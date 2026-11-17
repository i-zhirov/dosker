//! Build orchestration: base resolution, the instruction loop, and OCI commit.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use dosker_core::{BasePin, Doskerfile, EngineName, FromSource, Instruction, InstructionKind};
use dosker_image::freedos::{BaseInputs, FreedosVersion, ensure_freedos_base};
use dosker_image::{
    CacheKeyInput, CacheStore, Descriptor, DosPath, ImageConfig, ImageError, ImageMedia, ImageRef,
    ImageReference, Layer, OciStore, Tree, cache_key, fixed_timestamp,
};
use sha2::{Digest as _, Sha256};

use crate::cache::{BuildCache, CacheLookup};
use crate::context::BuildContext;
use crate::copy::{CopyPlan, plan_copy};
use crate::error::BuildError;
use crate::lock::{EngineResolver, HostEngineResolver, resolve_lock, write_lock};
use crate::normalize::normalize_instruction;
use crate::reporter::BuildReporter;
use crate::run_host::{HostRunRequest, run_host};
use crate::state::BuildState;

/// Options that control one build.
#[derive(Clone, Debug)]
pub struct BuildOptions {
    /// The tag applied to the committed image.
    pub tag: Option<ImageReference>,
    /// Whether the lock pins are refreshed instead of checked.
    pub update_lock: bool,
    /// Whether the build cache is bypassed.
    pub no_cache: bool,
    /// The time allowed for each `RUN --host` command.
    pub timeout: Duration,
    /// The clock recorded in new history entries.
    pub now: DateTime<Utc>,
}

/// The result of one successful build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildOutcome {
    /// The tag applied to the image, when one was given.
    pub reference: Option<ImageReference>,
    /// The digest of the committed manifest.
    pub manifest_digest: String,
    /// The digest of the committed config.
    pub config_digest: String,
    /// The number of instructions applied after `FROM`.
    pub steps: usize,
    /// The number of instructions served from the cache.
    pub cached_steps: usize,
}

/// The build orchestrator over one data root.
pub struct Builder {
    root: PathBuf,
    store: OciStore,
    cache: CacheStore,
    resolver: Box<dyn EngineResolver>,
    freedos_inputs: Option<BaseInputs>,
    agent_executable: Option<PathBuf>,
}

impl Builder {
    /// Creates a builder that resolves engines from the host.
    pub fn new(root: PathBuf) -> Self {
        let resolver = HostEngineResolver::new(root.clone());
        Self::with_resolver(root, Box::new(resolver))
    }

    /// Creates a builder that resolves engines through a supplied resolver.
    pub fn with_resolver(root: PathBuf, resolver: Box<dyn EngineResolver>) -> Self {
        Self {
            store: OciStore::new(root.clone()),
            cache: CacheStore::new(root.clone()),
            root,
            resolver,
            freedos_inputs: None,
            agent_executable: None,
        }
    }

    /// Supplies the pinned inputs the FreeDOS base is built from.
    pub fn with_freedos_inputs(mut self, inputs: BaseInputs) -> Self {
        self.freedos_inputs = Some(inputs);
        self
    }

    /// Selects the agent executable used by local imports.
    pub fn with_agent_executable(mut self, path: PathBuf) -> Self {
        self.agent_executable = Some(path);
        self
    }

    /// Builds the image described by the Doskerfile in `context_dir`.
    pub fn build(
        &self,
        context_dir: &Path,
        options: &BuildOptions,
        reporter: &mut dyn BuildReporter,
    ) -> Result<BuildOutcome, BuildError> {
        let context = BuildContext::open(context_dir)?;
        let doskerfile_path = context.doskerfile_path();
        let bytes = fs::read(&doskerfile_path).map_err(|source| BuildError::Io {
            path: doskerfile_path,
            source,
        })?;
        let doskerfile = Doskerfile::parse_bytes(&bytes)?;
        let source = from_source(&doskerfile)?;
        let base = self.resolve_base(&context, source)?;
        let selected_engine = select_engine(&doskerfile.instructions, &base.config);
        let resolved_engine = self.resolver.resolve(selected_engine.as_str())?;
        let lock = resolve_lock(
            context_dir,
            base.base_pin.clone(),
            base.config.agent_version.as_deref(),
            &resolved_engine,
            options.update_lock,
        )?;

        reporter
            .line(&format!("FROM {}", source.text()))
            .map_err(BuildError::Reporter)?;

        let ResolvedBase {
            config,
            tree,
            layers: base_layers,
            annotations,
            ..
        } = base;
        let mut state = BuildState::new(config, tree, &base_layers, selected_engine);
        let build_cache = BuildCache::new(&self.store, &self.cache, options.no_cache);
        let total = doskerfile.instructions.len().saturating_sub(1);
        let mut cached_steps = 0;

        for (step, instruction) in doskerfile.instructions.iter().enumerate().skip(1) {
            let prepared = prepare_step(&context, &state, instruction)?;
            let parent_tree_digest = state.parent_tree_digest()?;
            let config_digest = state.config_digest()?;
            let key = cache_key(&CacheKeyInput {
                parent_tree_digest: &parent_tree_digest,
                config_digest: &config_digest,
                instruction: &prepared.normalized,
                input_hashes: &prepared.input_hashes,
            });
            reporter
                .line(&format!("Step {step}/{total}: {}", prepared.normalized))
                .map_err(BuildError::Reporter)?;

            let previous = state.head_layer_digest.clone();
            let lookup = build_cache.lookup(
                &key,
                &prepared.normalized,
                previous.as_deref(),
                prepared.tree_step,
            )?;

            let cached = match lookup {
                CacheLookup::HitLayer { layer, descriptor } => {
                    state.apply_layer(layer, descriptor, &prepared.normalized, options.now)?;
                    true
                }
                CacheLookup::HitConfig => {
                    state.apply_config(instruction, &prepared.normalized, options.now)?;
                    true
                }
                CacheLookup::Miss => {
                    self.apply_miss(
                        &mut state,
                        instruction,
                        &prepared,
                        previous.as_deref(),
                        &build_cache,
                        &key,
                        options,
                        reporter,
                    )?;
                    false
                }
            };

            if cached {
                cached_steps += 1;
            }
            let status = if cached { "cached" } else { "built" };
            reporter
                .line(&format!("Step {step}/{total}: {status}"))
                .map_err(BuildError::Reporter)?;
        }

        let mut layers = base_layers;
        layers.extend(state.layers.iter().cloned());
        let committed =
            self.store
                .commit_image(options.tag.as_ref(), &state.config, layers, annotations)?;
        write_lock(context_dir, &lock)?;

        Ok(BuildOutcome {
            reference: options.tag.clone(),
            manifest_digest: committed.manifest_digest,
            config_digest: committed.config_digest,
            steps: total,
            cached_steps,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_miss(
        &self,
        state: &mut BuildState,
        instruction: &Instruction,
        prepared: &PreparedStep,
        previous: Option<&str>,
        cache: &BuildCache<'_>,
        key: &str,
        options: &BuildOptions,
        reporter: &mut dyn BuildReporter,
    ) -> Result<(), BuildError> {
        match &prepared.action {
            StepAction::Copy(plan) => {
                let layer = Layer::from_delta(
                    &plan.delta,
                    &prepared.normalized,
                    previous.map(str::to_owned),
                    None,
                )?;
                let descriptor = self.store.put_layer(&layer)?;
                state.apply_layer(layer, descriptor.clone(), &prepared.normalized, options.now)?;
                cache.store(key, &descriptor.digest, &prepared.normalized, options.now)?;
            }
            StepAction::RunHost { command, workdir } => {
                let tree = state
                    .tree
                    .as_ref()
                    .ok_or_else(|| instruction_error(instruction, "a booter image has no tree"))?;
                let request = HostRunRequest {
                    line: instruction.line,
                    command,
                    workdir,
                    env: &state.config.env,
                    timeout: options.timeout,
                };
                let delta = run_host(tree, &request, &self.root.join("tmp"), reporter)?;
                let layer = Layer::from_delta(
                    &delta,
                    &prepared.normalized,
                    previous.map(str::to_owned),
                    None,
                )?;
                let descriptor = self.store.put_layer(&layer)?;
                state.apply_layer(layer, descriptor.clone(), &prepared.normalized, options.now)?;
                cache.store(key, &descriptor.digest, &prepared.normalized, options.now)?;
            }
            StepAction::Config => {
                state.apply_config(instruction, &prepared.normalized, options.now)?;
                let layer_digest = state.head_layer_digest.clone().unwrap_or_default();
                cache.store(key, &layer_digest, &prepared.normalized, options.now)?;
            }
        }
        Ok(())
    }

    fn resolve_base(
        &self,
        context: &BuildContext,
        source: &FromSource,
    ) -> Result<ResolvedBase, BuildError> {
        match source {
            FromSource::Freedos { version } => {
                let version = FreedosVersion::parse(version)?;
                ensure_freedos_base(&self.store, self.freedos_inputs.as_ref(), &version)?;
                self.load_stored_base(&format!("freedos:{}", version.as_str()))
            }
            FromSource::Image(value) => self.load_stored_base(value),
            FromSource::LocalPath(path) => self.import_base(context.root(), path, ImageMedia::Disk),
            FromSource::BooterPath(path) => {
                self.import_base(context.root(), path, ImageMedia::Booter)
            }
        }
    }

    fn load_stored_base(&self, value: &str) -> Result<ResolvedBase, BuildError> {
        let reference = ImageRef::parse(value)?;
        let image = self.store.load(&reference)?;
        let manifest = self.store.load_manifest(&reference)?;
        let config = self.store.load_config(&reference)?;
        let tree = if config.booter {
            None
        } else {
            Some(self.store.load_tree(&reference)?)
        };
        let (name, tag) = base_identity(value, &reference);
        let annotations = manifest.annotations.clone().unwrap_or_default();
        Ok(ResolvedBase {
            config,
            tree,
            layers: manifest.layers,
            annotations,
            base_pin: BasePin {
                name,
                tag,
                digest: image.manifest_digest,
            },
        })
    }

    fn import_base(
        &self,
        context_root: &Path,
        source_path: &str,
        media: ImageMedia,
    ) -> Result<ResolvedBase, BuildError> {
        let path = if Path::new(source_path).is_absolute() {
            PathBuf::from(source_path)
        } else {
            context_root.join(source_path)
        };
        let source_digest = hash_file(&path)?;
        let agent = match media {
            ImageMedia::Disk => Some(self.load_agent()?),
            ImageMedia::Booter => None,
        };
        let reference = import_reference(media, &source_digest, agent.as_deref())?;
        let image_ref = ImageRef::Tag(reference.clone());

        let mut image = match self.store.load(&image_ref) {
            Ok(image) => Some(image),
            Err(ImageError::NotFound(_)) => None,
            Err(error) => return Err(error.into()),
        };
        let reusable = match image.as_ref() {
            Some(_) if media == ImageMedia::Disk => {
                self.is_reusable_disk_import(&image_ref, &source_digest)?
            }
            _ => true,
        };
        if !reusable {
            self.store.remove(&image_ref)?;
            image = None;
        }
        let image = match image {
            Some(image) => image,
            None => {
                match self.store.import(
                    &path,
                    &reference,
                    media,
                    agent.as_deref(),
                    fixed_timestamp(),
                ) {
                    Ok(image) => image,
                    Err(ImageError::AlreadyExists(_)) => self.store.load(&image_ref)?,
                    Err(error) => return Err(error.into()),
                }
            }
        };

        let manifest = self.store.load_manifest(&image_ref)?;
        let config = self.store.load_config(&image_ref)?;
        let tree = if config.booter {
            None
        } else {
            Some(self.store.load_tree(&image_ref)?)
        };
        if media == ImageMedia::Booter && !config.booter {
            return Err(BuildError::UnsupportedBase {
                reason: format!("{source_path} is not a booter image"),
            });
        }
        let annotations = manifest.annotations.clone().unwrap_or_default();
        Ok(ResolvedBase {
            config,
            tree,
            layers: manifest.layers,
            annotations,
            base_pin: BasePin {
                name: reference.name().as_str().to_owned(),
                tag: reference.tag().as_str().to_owned(),
                digest: image.manifest_digest,
            },
        })
    }

    fn load_agent(&self) -> Result<Vec<u8>, BuildError> {
        match &self.agent_executable {
            Some(path) => fs::read(path).map_err(|source| BuildError::Io {
                path: path.clone(),
                source,
            }),
            None => Ok(dosker_image::load_agent_executable()?),
        }
    }

    fn is_reusable_disk_import(
        &self,
        reference: &ImageRef,
        source_digest: &str,
    ) -> Result<bool, BuildError> {
        let manifest = self.store.load_manifest(reference)?;
        let config = self.store.load_config(reference)?;
        let Some(raw_digest) = manifest.raw_disk_digest() else {
            return Ok(false);
        };
        let Ok(raw_path) = self.store.blob_path(raw_digest) else {
            return Ok(false);
        };
        Ok(manifest.source_disk_digest() == Some(source_digest)
            && config.agent_version.as_deref() == Some(dosker_image::freedos::AGENT_VERSION)
            && raw_path.exists())
    }
}

struct ResolvedBase {
    config: ImageConfig,
    tree: Option<Tree>,
    layers: Vec<Descriptor>,
    annotations: BTreeMap<String, String>,
    base_pin: BasePin,
}

struct PreparedStep {
    normalized: String,
    input_hashes: Vec<String>,
    tree_step: bool,
    action: StepAction,
}

enum StepAction {
    Copy(CopyPlan),
    RunHost { command: String, workdir: DosPath },
    Config,
}

fn prepare_step(
    context: &BuildContext,
    state: &BuildState,
    instruction: &Instruction,
) -> Result<PreparedStep, BuildError> {
    let workdir = DosPath::parse(&state.config.workdir)?;
    match &instruction.kind {
        InstructionKind::From(_) => Err(instruction_error(
            instruction,
            "FROM is handled before the instruction loop",
        )),
        InstructionKind::Copy {
            sources,
            destination,
        } => {
            if state.config.booter {
                return Err(instruction_error(
                    instruction,
                    "a booter image does not support COPY",
                ));
            }
            let tree = state
                .tree
                .as_ref()
                .ok_or_else(|| instruction_error(instruction, "a booter image has no tree"))?;
            let plan = plan_copy(
                context,
                tree,
                &workdir,
                sources,
                destination,
                instruction.line,
            )?;
            Ok(PreparedStep {
                normalized: plan.normalized_instruction.clone(),
                input_hashes: plan.input_hashes.clone(),
                tree_step: true,
                action: StepAction::Copy(plan),
            })
        }
        InstructionKind::Run {
            host: true, raw, ..
        } => {
            if state.config.booter {
                return Err(instruction_error_named(
                    instruction,
                    "RUN --host",
                    "a booter image does not support RUN --host",
                ));
            }
            let normalized = normalize_instruction(instruction, &workdir)?;
            let command = raw.clone();
            let staged = state.parent_tree_digest()?;
            let input_hashes = vec![
                format!("context:{}", context.digest()),
                format!("staged:{staged}"),
            ];
            Ok(PreparedStep {
                normalized,
                input_hashes,
                tree_step: true,
                action: StepAction::RunHost { command, workdir },
            })
        }
        InstructionKind::Run { host: false, .. } => Err(instruction_error(
            instruction,
            "in-image RUN is not supported; use RUN --host",
        )),
        _ => {
            let normalized = normalize_instruction(instruction, &workdir)
                .map_err(|error| instruction_error(instruction, error.to_string()))?;
            Ok(PreparedStep {
                normalized,
                input_hashes: Vec::new(),
                tree_step: false,
                action: StepAction::Config,
            })
        }
    }
}

fn from_source(doskerfile: &Doskerfile) -> Result<&FromSource, BuildError> {
    match doskerfile
        .instructions
        .first()
        .map(|instruction| &instruction.kind)
    {
        Some(InstructionKind::From(source)) => Ok(source),
        _ => Err(BuildError::Context {
            reason: "the Doskerfile has no FROM instruction".to_owned(),
        }),
    }
}

fn select_engine(instructions: &[Instruction], config: &ImageConfig) -> EngineName {
    instructions
        .iter()
        .rev()
        .find_map(|instruction| match &instruction.kind {
            InstructionKind::Engine { name } => Some(*name),
            _ => None,
        })
        .or_else(|| config.engine_pin.as_deref().and_then(EngineName::parse))
        .unwrap_or(EngineName::Qemu)
}

fn base_identity(value: &str, reference: &ImageRef) -> (String, String) {
    if let Some((name, _)) = value.split_once('@') {
        return (name.to_owned(), String::new());
    }
    match reference {
        ImageRef::Tag(tag) => (
            tag.name().as_str().to_owned(),
            tag.tag().as_str().to_owned(),
        ),
        ImageRef::Digest(_) => (String::new(), String::new()),
    }
}

fn import_reference(
    media: ImageMedia,
    source_digest: &str,
    agent: Option<&[u8]>,
) -> Result<ImageReference, BuildError> {
    let key = match (media, agent) {
        (ImageMedia::Disk, Some(bytes)) => {
            let agent_digest = dosker_image::sha256_digest(bytes);
            dosker_image::sha256_digest(format!("{source_digest}\n{agent_digest}").as_bytes())
        }
        _ => source_digest.to_owned(),
    };
    let hex = key.strip_prefix("sha256:").unwrap_or(&key);
    Ok(ImageReference::parse(&format!("dosker-build:{hex}"))?)
}

fn hash_file(path: &Path) -> Result<String, BuildError> {
    let mut file = fs::File::open(path).map_err(|source| BuildError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|source| BuildError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", hex(&hasher.finalize())))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut text, "{byte:02x}").expect("writing to a String cannot fail");
    }
    text
}

fn instruction_error(instruction: &Instruction, reason: impl Into<String>) -> BuildError {
    instruction_error_named(instruction, instruction.kind.name(), reason)
}

fn instruction_error_named(
    instruction: &Instruction,
    name: &str,
    reason: impl Into<String>,
) -> BuildError {
    BuildError::Instruction {
        line: instruction.line,
        instruction: name.to_owned(),
        reason: reason.into(),
    }
}
