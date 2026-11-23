//! Stores the FreeDOS base and reuses it by tag.

use std::collections::BTreeMap;

use crate::boot::BootMetadata;
use crate::config::{DosFlavour, HistoryEntry, ImageConfig, MachineProfile};
use crate::error::ImageError;
use crate::freedos::AGENT_VERSION;
use crate::freedos::build::{BaseImage, build_base_image, validate_base_boot};
use crate::freedos::inputs::{BaseInputs, FreedosVersion, load_freedos_binaries};
use crate::layer::{Layer, fixed_timestamp};
use crate::licenses::freedos_base_licenses;
use crate::manifest::{ANNOTATION_RAW_DISK_DIGEST, Manifest};
use crate::oci::{CommittedImage, Image, OciStore};
use crate::reference::{ImageRef, ImageReference};

/// The config every stored FreeDOS base must carry.
pub(crate) fn expected_base_config(boot: &BootMetadata, agent_version: &str) -> ImageConfig {
    ImageConfig {
        schema_version: 1,
        dos_flavour: DosFlavour::Freedos,
        booter: false,
        boot: boot.clone(),
        machine: MachineProfile::default(),
        env: BTreeMap::new(),
        workdir: r"C:\".to_owned(),
        cmd: None,
        exposed_ports: Vec::new(),
        volumes: Vec::new(),
        engine_pin: None,
        engine_config: BTreeMap::new(),
        requires: Vec::new(),
        agent_version: Some(agent_version.to_owned()),
        created_by: "freedos-base".to_owned(),
        history: vec![HistoryEntry {
            created_by: "freedos-base".to_owned(),
            created_at: fixed_timestamp(),
            empty_layer: false,
            comment: None,
        }],
        licenses: freedos_base_licenses(),
    }
}

impl BaseImage {
    /// Builds the image config that identifies this base.
    pub fn base_config(&self) -> ImageConfig {
        expected_base_config(&self.boot, &self.agent_version)
    }

    /// Commits the tree layer, the raw disk, and the config under a reference.
    pub fn store(
        &self,
        store: &OciStore,
        reference: &ImageReference,
    ) -> Result<CommittedImage, ImageError> {
        let config = self.base_config();
        let layer = Layer::from_delta(&self.tree.to_delta(), "freedos-base", None, None)?;
        let descriptor = store.put_layer(&layer)?;
        let raw = store.put_blob(&self.raw)?;
        let mut annotations = BTreeMap::new();
        annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), raw.digest);
        store.commit_image(Some(reference), &config, vec![descriptor], annotations)
    }
}

/// Returns the stored base, building and tagging it only when it is missing or stale.
pub fn ensure_freedos_base(
    store: &OciStore,
    inputs: Option<&BaseInputs>,
    version: &FreedosVersion,
) -> Result<Image, ImageError> {
    let reference = ImageReference::parse(&format!("freedos:{}", version.as_str()))?;
    let tag = ImageRef::Tag(reference.clone());
    if let (Ok(config), Ok(manifest)) = (store.load_config(&tag), store.load_manifest(&tag))
        && reusable(store, &config, &manifest)
    {
        return store.load(&tag);
    }

    let owned_inputs = if inputs.is_none() {
        Some(BaseInputs::from_environment(version)?)
    } else {
        None
    };
    let inputs = inputs.unwrap_or_else(|| owned_inputs.as_ref().expect("the inputs were loaded"));
    let binaries = load_freedos_binaries(inputs)?;
    let image = build_base_image(&binaries, version)?;
    image.store(store, &reference)?;
    store.load(&tag)
}

fn reusable(store: &OciStore, config: &ImageConfig, manifest: &Manifest) -> bool {
    if *config != expected_base_config(&config.boot, AGENT_VERSION) {
        return false;
    }
    if validate_base_boot(&config.boot).is_err() {
        return false;
    }
    let Some(raw) = manifest.raw_disk_digest() else {
        return false;
    };
    store
        .blob_path(raw)
        .map(|path| path.exists())
        .unwrap_or(false)
}
