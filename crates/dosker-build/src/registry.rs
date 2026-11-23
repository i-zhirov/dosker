//! Registry metadata, namespaces, and validation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use dosker_core::{
    Doskerfile, FromSource, InstructionKind, LicenseEntry, LicenseStatus, LicenseSummary,
    license_summary, merge_license_entry, validate_licenses,
};
use dosker_image::{
    DosPath, ImageDigest, ImageError, ImageRef, ImageTag, Namespace, OciStore, ReferenceSelector,
    RegistryReference, Tree, TreeNode,
};
use serde::{Deserialize, Serialize};

use crate::context::BuildContext;
use crate::copy::plan_copy;
use crate::normalize::resolve_dos_path;

/// The notice file required for copyleft or abandonware content.
pub const NOTICE_PATH: &str = r"C:\DOSKER\NOTICE.TXT";

/// Top-level directories that hold the DOSker system content.
pub const SYSTEM_TOP_LEVEL: [&str; 2] = ["DOS", "DOSKER"];

/// Root files that hold the DOSker system content.
pub const SYSTEM_ROOT_FILES: [&str; 5] = [
    "KERNEL.SYS",
    "COMMAND.COM",
    "CONFIG.SYS",
    "AUTOEXEC.BAT",
    "AUTOEXEC.OLD",
];

/// Extensions that identify an executable file.
pub const EXECUTABLE_EXTENSIONS: [&str; 3] = ["EXE", "COM", "BAT"];

/// One image's `registry.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryMetadata {
    pub schema: u32,
    pub version: String,
    pub tags: Vec<String>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<String>,
}

/// The registry-wide `namespaces.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Namespaces {
    pub schema: u32,
    pub owners: BTreeMap<String, NamespaceOwner>,
}

/// One proven namespace owner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamespaceOwner {
    pub github: String,
    pub proof: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A registry validation failure.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The metadata file is malformed or invalid.
    #[error("registry metadata at {}: {reason}", path.display())]
    Metadata { path: PathBuf, reason: String },

    /// The namespaces file is malformed or invalid.
    #[error("namespaces at {}: {reason}", path.display())]
    Namespaces { path: PathBuf, reason: String },

    /// Two images declare the same tag.
    #[error("duplicate tag {tag}: {} and {}", first.display(), second.display())]
    DuplicateTag {
        tag: String,
        first: PathBuf,
        second: PathBuf,
    },

    /// The image owner is not in `namespaces.toml`.
    #[error("unknown namespace owner: {owner}")]
    UnknownOwner { owner: String },

    /// The owner has no valid proof URL.
    #[error("namespace owner {owner} has no valid proof URL")]
    UnprovenOwner { owner: String },

    /// The base image is not in the local store.
    #[error("base image {reference} is not available locally; pull it first")]
    BaseNotLocal { reference: String },

    /// A booter image does not declare `COVERS=*`.
    #[error("booter image must declare COVERS=*")]
    BooterNeedsWildcard,

    /// A booter image needs a notice that cannot be inspected.
    #[error("booter image {image} needs a notice but its tree cannot be inspected")]
    BooterNoticeUnverifiable { image: String },

    /// The final image is not in the local store.
    #[error("image {reference} is not available locally; build it first")]
    FinalNotLocal { reference: String },

    /// The final image failed a license, coverage, or notice check.
    #[error("final image {image}: {reason}")]
    Final { image: String, reason: String },

    /// A filesystem operation failed.
    #[error("i/o error at {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    /// The Doskerfile is invalid or not buildable.
    #[error("Doskerfile at {}: {reason}", path.display())]
    Doskerfile { path: PathBuf, reason: String },

    /// No registry root contains the image directory.
    #[error("no registry root above {}", path.display())]
    RootNotFound { path: PathBuf },
}

/// Parses one `registry.toml`.
pub fn parse_metadata(path: &Path) -> Result<RegistryMetadata, RegistryError> {
    let text = fs::read_to_string(path).map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|error| RegistryError::Metadata {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

/// Parses one `namespaces.toml`.
pub fn parse_namespaces(path: &Path) -> Result<Namespaces, RegistryError> {
    let text = fs::read_to_string(path).map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&text).map_err(|error| RegistryError::Namespaces {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

/// Validates one `registry.toml`.
pub fn validate_metadata(metadata: &RegistryMetadata, path: &Path) -> Result<(), RegistryError> {
    let fail = |reason: String| {
        Err(RegistryError::Metadata {
            path: path.to_path_buf(),
            reason,
        })
    };

    if metadata.schema != 1 {
        return fail(format!("schema must be 1, found {}", metadata.schema));
    }
    if ImageTag::parse(&metadata.version).is_err() {
        return fail(format!("version {:?} is not a valid tag", metadata.version));
    }
    if metadata.tags.is_empty() {
        return fail("tags must not be empty".to_owned());
    }
    if metadata.tags.len() > 32 {
        return fail("tags must have at most 32 entries".to_owned());
    }

    let mut seen = BTreeSet::new();
    for tag in &metadata.tags {
        if ImageTag::parse(tag).is_err() {
            return fail(format!("tag {tag:?} is not a valid tag"));
        }
        if !seen.insert(tag.as_str()) {
            return fail(format!("tag {tag:?} is duplicated"));
        }
    }

    if !(1..=280).contains(&metadata.description.chars().count()) {
        return fail("description must have 1 to 280 characters".to_owned());
    }
    if metadata
        .homepage
        .as_deref()
        .is_some_and(|homepage| !is_http_url(homepage))
    {
        return fail("homepage must be an http or https URL".to_owned());
    }
    if metadata
        .deprecated
        .as_deref()
        .is_some_and(|deprecated| !(1..=280).contains(&deprecated.chars().count()))
    {
        return fail("deprecated must have 1 to 280 characters".to_owned());
    }
    Ok(())
}

/// Validates the namespace file and requires `owner` to be proven.
pub fn validate_namespaces(
    namespaces: &Namespaces,
    path: &Path,
    owner: &str,
) -> Result<(), RegistryError> {
    let fail = |reason: String| {
        Err(RegistryError::Namespaces {
            path: path.to_path_buf(),
            reason,
        })
    };

    if namespaces.schema != 1 {
        return fail(format!("schema must be 1, found {}", namespaces.schema));
    }
    if namespaces.owners.is_empty() {
        return fail("owners must not be empty".to_owned());
    }

    for (key, entry) in &namespaces.owners {
        if let Err(error) = Namespace::parse(key) {
            return fail(format!("invalid owner key {key:?}: {error}"));
        }
        if entry.github.as_str() != key.as_str() {
            return fail(format!("owner {key:?} must have github {key:?}"));
        }
        if !is_http_url(&entry.proof) {
            return Err(RegistryError::UnprovenOwner { owner: key.clone() });
        }
        if entry
            .note
            .as_deref()
            .is_some_and(|note| note.chars().count() > 280)
        {
            return fail(format!("owner {key:?} note is longer than 280 characters"));
        }
    }

    if !namespaces.owners.contains_key(owner) {
        return Err(RegistryError::UnknownOwner {
            owner: owner.to_owned(),
        });
    }
    Ok(())
}

/// Collects every tag in `images/*/*/registry.toml` and rejects duplicates.
pub fn scan_tags(root: &Path) -> Result<BTreeMap<String, PathBuf>, RegistryError> {
    let images = root.join("images");
    let mut tags = BTreeMap::new();

    for owner in read_dir(&images)? {
        if !is_dir(&owner)? {
            continue;
        }
        for image in read_dir(&owner.path())? {
            if !is_dir(&image)? {
                continue;
            }
            let metadata_path = image.path().join("registry.toml");
            let metadata = parse_metadata(&metadata_path)?;
            validate_metadata(&metadata, &metadata_path)?;
            for tag in metadata.tags {
                if let Some(first) = tags.insert(tag.clone(), metadata_path.clone()) {
                    return Err(RegistryError::DuplicateTag {
                        tag,
                        first,
                        second: metadata_path,
                    });
                }
            }
        }
    }

    Ok(tags)
}

/// Finds the registry root for an image directory.
pub fn registry_root(image_dir: &Path, explicit: Option<&Path>) -> Result<PathBuf, RegistryError> {
    if let Some(root) = explicit {
        return Ok(root.to_path_buf());
    }

    let mut current = Some(image_dir);
    while let Some(dir) = current {
        if dir.join("images").is_dir() {
            return Ok(dir.to_path_buf());
        }
        current = dir.parent();
    }

    Err(RegistryError::RootNotFound {
        path: image_dir.to_path_buf(),
    })
}

/// Matches a DOS path against a `*`/`?` glob, ignoring case.
pub fn covers_glob(pattern: &str, path: &str) -> bool {
    let pattern = pattern.as_bytes();
    let path = path.as_bytes();
    let mut pattern_index = 0;
    let mut path_index = 0;
    let mut star = None;
    let mut star_path = 0;

    while path_index < path.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?'
                || pattern[pattern_index].eq_ignore_ascii_case(&path[path_index]))
        {
            pattern_index += 1;
            path_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star = Some(pattern_index);
            pattern_index += 1;
            star_path = path_index;
        } else if let Some(star_index) = star {
            pattern_index = star_index + 1;
            star_path += 1;
            path_index = star_path;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

/// Returns whether a glob covers a top-level directory.
pub fn covers_directory(pattern: &str, dir: &str) -> bool {
    covers_glob(pattern, dir) || pattern.eq_ignore_ascii_case(&format!("{dir}\\*"))
}

/// Returns the uncovered top-level directories and root executables.
pub fn coverage_misses(
    tree: &Tree,
    entries: &[LicenseEntry],
    booter: bool,
) -> Result<Vec<String>, RegistryError> {
    let covers = entries
        .iter()
        .flat_map(|entry| entry.covers.iter())
        .map(String::as_str)
        .collect::<Vec<_>>();

    if booter {
        if covers.contains(&"*") {
            return Ok(Vec::new());
        }
        return Err(RegistryError::BooterNeedsWildcard);
    }

    let mut misses = Vec::new();
    for dir in tree.dirs() {
        if dir.path.components().count() != 1 || is_system_top_level(&dir.path) {
            continue;
        }
        let path = dir.path.to_dos_string();
        if !covers.iter().any(|cover| covers_directory(cover, &path)) {
            misses.push(path);
        }
    }
    for file in tree.files() {
        if file.path.components().count() != 1
            || is_system_root_file(&file.path)
            || !is_executable(&file.path)
        {
            continue;
        }
        let path = file.path.to_dos_string();
        if !covers.iter().any(|cover| covers_glob(cover, &path)) {
            misses.push(path);
        }
    }
    Ok(misses)
}

/// Returns whether any entry requires the notice file.
pub fn requires_notice(entries: &[LicenseEntry]) -> bool {
    entries
        .iter()
        .any(|entry| entry.is_copyleft() || entry.status == LicenseStatus::Abandonware)
}

/// Returns whether the tree contains the notice file.
pub fn notice_present(tree: &Tree) -> bool {
    let path = DosPath::parse(NOTICE_PATH).expect("the notice path is valid");
    matches!(tree.get(&path), Some(TreeNode::File(_)))
}

fn is_system_top_level(path: &DosPath) -> bool {
    path.components().next().is_some_and(|component| {
        SYSTEM_TOP_LEVEL
            .iter()
            .any(|system| component.eq_ignore_ascii_case(system))
    })
}

fn is_system_root_file(path: &DosPath) -> bool {
    path.file_name().is_some_and(|name| {
        SYSTEM_ROOT_FILES
            .iter()
            .any(|system| name.eq_ignore_ascii_case(system))
    })
}

fn is_executable(path: &DosPath) -> bool {
    path.file_name()
        .and_then(|name| name.rsplit_once('.'))
        .is_some_and(|(_, extension)| {
            EXECUTABLE_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

/// The result of validating one image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageValidation {
    /// The image path relative to the registry root.
    pub image: String,
    /// The immutable version from `registry.toml`.
    pub version: String,
    /// The movable tags from `registry.toml`.
    pub tags: Vec<String>,
    /// The license summary of the effective entries.
    pub license_summary: LicenseSummary,
    /// The tree paths that were checked.
    pub checked_paths: Vec<String>,
}

/// One validation target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryTarget {
    /// A registry root that contains `images/`.
    Root(PathBuf),
    /// One image directory and its registry root.
    Image { root: PathBuf, image: PathBuf },
}

/// Resolves a path to a registry root or one image directory.
pub fn resolve_target(
    path: &Path,
    explicit_root: Option<&Path>,
) -> Result<RegistryTarget, RegistryError> {
    if path.join("images").is_dir() {
        return Ok(RegistryTarget::Root(path.to_path_buf()));
    }
    let root = registry_root(path, explicit_root)?;
    Ok(RegistryTarget::Image {
        root,
        image: path.to_path_buf(),
    })
}

/// Validates a root or one image directory.
pub fn validate_target(
    target: &RegistryTarget,
    store: &OciStore,
) -> Result<Vec<ImageValidation>, RegistryError> {
    match target {
        RegistryTarget::Root(root) => validate_registry(root, store),
        RegistryTarget::Image { root, image } => {
            let namespaces_path = root.join("namespaces.toml");
            let namespaces = parse_namespaces(&namespaces_path)?;
            let owner = owner_of(image)?;
            validate_namespaces(&namespaces, &namespaces_path, &owner)?;
            scan_tags(root)?;
            Ok(vec![validate_image(root, image, store)?])
        }
    }
}

/// Validates every image below one registry root.
pub fn validate_registry(
    root: &Path,
    store: &OciStore,
) -> Result<Vec<ImageValidation>, RegistryError> {
    let namespaces_path = root.join("namespaces.toml");
    let namespaces = parse_namespaces(&namespaces_path)?;
    scan_tags(root)?;

    let mut validations = Vec::new();
    for image in image_dirs(root)? {
        let owner = owner_of(&image)?;
        validate_namespaces(&namespaces, &namespaces_path, &owner)?;
        validations.push(validate_image(root, &image, store)?);
    }
    Ok(validations)
}

/// Validates one image directory against the registry rules.
pub fn validate_image(
    root: &Path,
    image_dir: &Path,
    store: &OciStore,
) -> Result<ImageValidation, RegistryError> {
    let metadata_path = image_dir.join("registry.toml");
    let metadata = parse_metadata(&metadata_path)?;
    validate_metadata(&metadata, &metadata_path)?;

    let context = BuildContext::open(image_dir).map_err(|error| RegistryError::Doskerfile {
        path: image_dir.to_path_buf(),
        reason: error.to_string(),
    })?;
    let doskerfile_path = context.doskerfile_path();
    let bytes = fs::read(&doskerfile_path).map_err(|source| RegistryError::Io {
        path: doskerfile_path.clone(),
        source,
    })?;
    let doskerfile =
        Doskerfile::parse_bytes(&bytes).map_err(|error| RegistryError::Doskerfile {
            path: doskerfile_path.clone(),
            reason: error.to_string(),
        })?;
    let source = match doskerfile
        .instructions
        .first()
        .map(|instruction| &instruction.kind)
    {
        Some(InstructionKind::From(source)) => source,
        _ => {
            return Err(RegistryError::Doskerfile {
                path: doskerfile_path,
                reason: "the Doskerfile has no FROM instruction".to_owned(),
            });
        }
    };

    let (mut tree, mut workdir, mut licenses, booter) =
        resolve_base(source, &context, &doskerfile_path, store)?;

    for instruction in doskerfile.instructions.iter().skip(1) {
        match &instruction.kind {
            InstructionKind::Copy {
                sources,
                destination,
            } => {
                let tree = tree.as_mut().ok_or_else(|| RegistryError::Doskerfile {
                    path: doskerfile_path.clone(),
                    reason: "a booter image has no tree".to_owned(),
                })?;
                let plan = plan_copy(
                    &context,
                    tree,
                    &workdir,
                    sources,
                    destination,
                    instruction.line,
                )
                .map_err(|error| RegistryError::Doskerfile {
                    path: doskerfile_path.clone(),
                    reason: error.to_string(),
                })?;
                tree.apply_delta(&plan.delta)
                    .map_err(|error| RegistryError::Doskerfile {
                        path: doskerfile_path.clone(),
                        reason: error.to_string(),
                    })?;
            }
            InstructionKind::Workdir { path } => {
                workdir = resolve_dos_path(&workdir, path).map_err(|error| {
                    RegistryError::Doskerfile {
                        path: doskerfile_path.clone(),
                        reason: error.to_string(),
                    }
                })?;
            }
            InstructionKind::License(entry) => {
                merge_license_entry(&mut licenses, entry.clone()).map_err(|error| {
                    RegistryError::Doskerfile {
                        path: doskerfile_path.clone(),
                        reason: error.to_string(),
                    }
                })?;
            }
            _ => {}
        }
    }

    if licenses.is_empty() {
        return Err(RegistryError::Doskerfile {
            path: doskerfile_path,
            reason: "the image has no license entries".to_owned(),
        });
    }
    validate_licenses(&licenses).map_err(|error| RegistryError::Doskerfile {
        path: doskerfile_path.clone(),
        reason: error.to_string(),
    })?;

    let misses = match &tree {
        Some(tree) => coverage_misses(tree, &licenses, false)?,
        None => coverage_misses(&Tree::new(), &licenses, true)?,
    };
    if !misses.is_empty() {
        return Err(RegistryError::Doskerfile {
            path: doskerfile_path,
            reason: format!("the image does not cover: {}", misses.join(", ")),
        });
    }

    if requires_notice(&licenses) {
        if booter {
            return Err(RegistryError::BooterNoticeUnverifiable {
                image: image_label(root, image_dir),
            });
        }
        let tree = tree.as_ref().expect("a non-booter has a tree");
        if !notice_present(tree) {
            return Err(RegistryError::Doskerfile {
                path: doskerfile_path,
                reason: format!("the image needs a notice at {NOTICE_PATH}"),
            });
        }
    }

    let checked_paths = tree
        .as_ref()
        .map(|tree| {
            tree.files()
                .map(|file| file.path.to_dos_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ImageValidation {
        image: image_label(root, image_dir),
        version: metadata.version,
        tags: metadata.tags,
        license_summary: license_summary(&licenses).expect("the image has license entries"),
        checked_paths,
    })
}

/// Validates the final tree of a built image against its config licenses.
///
/// `publish` runs this after the build and the `RUN` steps. It reads the
/// stored config and tree, so it sees the final files and never the
/// Doskerfile.
pub fn validate_final(
    reference: &ImageRef,
    store: &OciStore,
) -> Result<ImageValidation, RegistryError> {
    let label = reference.to_string();
    let config = match store.load_config(reference) {
        Ok(config) => config,
        Err(ImageError::NotFound(_)) => {
            return Err(RegistryError::FinalNotLocal { reference: label });
        }
        Err(error) => {
            return Err(RegistryError::Final {
                image: label,
                reason: error.to_string(),
            });
        }
    };

    let tree = if config.booter {
        None
    } else {
        Some(
            store
                .load_tree(reference)
                .map_err(|error| RegistryError::Final {
                    image: label.clone(),
                    reason: error.to_string(),
                })?,
        )
    };

    let licenses = config.licenses;
    if licenses.is_empty() {
        return Err(RegistryError::Final {
            image: label,
            reason: "the image has no license entries".to_owned(),
        });
    }
    validate_licenses(&licenses).map_err(|error| RegistryError::Final {
        image: label.clone(),
        reason: error.to_string(),
    })?;

    let misses = match &tree {
        Some(tree) => coverage_misses(tree, &licenses, false)?,
        None => coverage_misses(&Tree::new(), &licenses, true)?,
    };
    if !misses.is_empty() {
        return Err(RegistryError::Final {
            image: label,
            reason: format!("the image does not cover: {}", misses.join(", ")),
        });
    }

    if requires_notice(&licenses) {
        if config.booter {
            return Err(RegistryError::BooterNoticeUnverifiable { image: label });
        }
        let tree = tree.as_ref().expect("a non-booter has a tree");
        if !notice_present(tree) {
            return Err(RegistryError::Final {
                image: label,
                reason: format!("the image needs a notice at {NOTICE_PATH}"),
            });
        }
    }

    let checked_paths = tree
        .as_ref()
        .map(|tree| {
            tree.files()
                .map(|file| file.path.to_dos_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ImageValidation {
        image: label.clone(),
        version: label,
        tags: Vec::new(),
        license_summary: license_summary(&licenses).expect("the image has license entries"),
        checked_paths,
    })
}

fn resolve_base(
    source: &FromSource,
    context: &BuildContext,
    doskerfile_path: &Path,
    store: &OciStore,
) -> Result<(Option<Tree>, DosPath, Vec<LicenseEntry>, bool), RegistryError> {
    let root = DosPath::parse("").expect("the root path is valid");
    match source {
        FromSource::Freedos { .. } => Ok((
            Some(Tree::new()),
            root,
            dosker_image::freedos_base_licenses(),
            false,
        )),
        FromSource::Image(value) => {
            let reference = image_reference(value).map_err(|error| RegistryError::Doskerfile {
                path: doskerfile_path.to_path_buf(),
                reason: error.to_string(),
            })?;
            let config = match store.load_config(&reference) {
                Ok(config) => config,
                Err(ImageError::NotFound(_)) => {
                    return Err(RegistryError::BaseNotLocal {
                        reference: value.clone(),
                    });
                }
                Err(error) => {
                    return Err(RegistryError::Doskerfile {
                        path: doskerfile_path.to_path_buf(),
                        reason: error.to_string(),
                    });
                }
            };
            let tree =
                if config.booter {
                    None
                } else {
                    Some(store.load_tree(&reference).map_err(|error| {
                        RegistryError::Doskerfile {
                            path: doskerfile_path.to_path_buf(),
                            reason: error.to_string(),
                        }
                    })?)
                };
            let workdir =
                DosPath::parse(&config.workdir).map_err(|error| RegistryError::Doskerfile {
                    path: doskerfile_path.to_path_buf(),
                    reason: error.to_string(),
                })?;
            Ok((tree, workdir, config.licenses, config.booter))
        }
        FromSource::LocalPath(path) => {
            let source_path = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                context.root().join(path)
            };
            let tree = dosker_image::read_tree(&source_path).map_err(|error| {
                RegistryError::Doskerfile {
                    path: doskerfile_path.to_path_buf(),
                    reason: error.to_string(),
                }
            })?;
            Ok((Some(tree), root, Vec::new(), false))
        }
        FromSource::BooterPath(_) => Ok((None, root, Vec::new(), true)),
    }
}

fn image_reference(value: &str) -> Result<ImageRef, ImageError> {
    if ImageDigest::parse(value).is_ok() {
        return ImageRef::parse(value);
    }
    let registry = RegistryReference::parse(value)?;
    match registry.selector() {
        ReferenceSelector::Tag(_) => Ok(ImageRef::Tag(
            registry
                .local_reference()
                .expect("a tag selector has a local reference"),
        )),
        ReferenceSelector::Digest(digest) => Ok(ImageRef::Digest(digest.clone())),
    }
}

fn image_dirs(root: &Path) -> Result<Vec<PathBuf>, RegistryError> {
    let images = root.join("images");
    let mut result = Vec::new();
    for owner in read_dir(&images)? {
        if !is_dir(&owner)? {
            continue;
        }
        for image in read_dir(&owner.path())? {
            if is_dir(&image)? {
                result.push(image.path());
            }
        }
    }
    result.sort();
    Ok(result)
}

fn owner_of(image: &Path) -> Result<String, RegistryError> {
    image
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| RegistryError::Doskerfile {
            path: image.to_path_buf(),
            reason: "the image path has no owner".to_owned(),
        })
}

fn image_label(root: &Path, image_dir: &Path) -> String {
    image_dir
        .strip_prefix(root)
        .unwrap_or(image_dir)
        .to_string_lossy()
        .replace('\\', "/")
}

fn read_dir(path: &Path) -> Result<Vec<fs::DirEntry>, RegistryError> {
    let entries = fs::read_dir(path).map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    entries
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| RegistryError::Io {
            path: path.to_path_buf(),
            source,
        })
}

fn is_dir(entry: &fs::DirEntry) -> Result<bool, RegistryError> {
    entry
        .file_type()
        .map(|kind| kind.is_dir())
        .map_err(|source| RegistryError::Io {
            path: entry.path(),
            source,
        })
}

fn is_http_url(value: &str) -> bool {
    value.starts_with("https://") || value.starts_with("http://")
}
