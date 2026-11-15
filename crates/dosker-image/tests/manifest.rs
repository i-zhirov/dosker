use dosker_image::{
    ANNOTATION_RAW_DISK_DIGEST, ANNOTATION_REF_NAME, Descriptor, Index, MEDIA_TYPE_BOOTER,
    MEDIA_TYPE_CONFIG, MEDIA_TYPE_INDEX, MEDIA_TYPE_LAYER, MEDIA_TYPE_MANIFEST, Manifest,
    OCI_LAYOUT_VERSION, OciLayout,
};
use std::collections::BTreeMap;

fn digest(value: &str) -> String {
    dosker_image::sha256_digest(value.as_bytes())
}

fn descriptor(media_type: &str, digest: &str, size: u64) -> Descriptor {
    Descriptor {
        media_type: media_type.to_owned(),
        digest: digest.to_owned(),
        size,
        annotations: None,
    }
}

fn tree_manifest() -> Manifest {
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_RAW_DISK_DIGEST.to_owned(), digest("disk"));
    Manifest::new(
        descriptor(MEDIA_TYPE_CONFIG, &digest("config"), 10),
        vec![descriptor(MEDIA_TYPE_LAYER, &digest("layer"), 20)],
        Some(annotations),
    )
}

#[test]
fn media_type_constants_match_the_contract() {
    assert_eq!(MEDIA_TYPE_LAYER, "application/vnd.dosker.layer.v1.tar");
    assert_eq!(
        MEDIA_TYPE_CONFIG,
        "application/vnd.dosker.image.config.v1+json"
    );
    assert_eq!(MEDIA_TYPE_BOOTER, "application/vnd.dosker.booter.v1.raw");
    assert_eq!(
        MEDIA_TYPE_MANIFEST,
        "application/vnd.oci.image.manifest.v1+json"
    );
    assert_eq!(MEDIA_TYPE_INDEX, "application/vnd.oci.image.index.v1+json");
    assert_eq!(ANNOTATION_REF_NAME, "org.opencontainers.image.ref.name");
    assert_eq!(ANNOTATION_RAW_DISK_DIGEST, "org.dosker.rawdisk.digest");
}

#[test]
fn manifest_serializes_standard_oci_fields() {
    let json = serde_json::to_value(tree_manifest()).unwrap();
    assert_eq!(json["schemaVersion"], 2);
    assert_eq!(json["mediaType"], MEDIA_TYPE_MANIFEST);
    assert_eq!(json["config"]["mediaType"], MEDIA_TYPE_CONFIG);
    assert_eq!(json["layers"][0]["mediaType"], MEDIA_TYPE_LAYER);
    assert!(json["annotations"].is_object());
}

#[test]
fn manifest_omits_absent_annotations() {
    let manifest = Manifest::new(
        descriptor(MEDIA_TYPE_CONFIG, &digest("config"), 10),
        vec![descriptor(MEDIA_TYPE_LAYER, &digest("layer"), 20)],
        None,
    );
    let json = serde_json::to_value(manifest).unwrap();
    assert!(json.get("annotations").is_none());
    assert!(json["config"].get("annotations").is_none());
    assert!(json["layers"][0].get("annotations").is_none());
}

#[test]
fn descriptor_omits_absent_annotations() {
    let json = serde_json::to_value(descriptor(MEDIA_TYPE_CONFIG, &digest("config"), 10)).unwrap();
    assert!(json.get("annotations").is_none());
}

#[test]
fn index_serializes_standard_oci_fields() {
    let index = Index::new();
    let json = serde_json::to_value(index).unwrap();
    assert_eq!(json["schemaVersion"], 2);
    assert_eq!(json["mediaType"], MEDIA_TYPE_INDEX);
}

#[test]
fn raw_disk_annotation_round_trips() {
    let manifest = tree_manifest();
    assert_eq!(manifest.raw_disk_digest(), Some(digest("disk").as_str()));
}

#[test]
fn manifest_validate_rejects_a_tree_image_without_layers() {
    let mut manifest = tree_manifest();
    manifest.layers.clear();
    assert!(manifest.validate(false).is_err());
}

#[test]
fn manifest_validate_rejects_a_booter_layer_on_a_tree_image() {
    let mut manifest = tree_manifest();
    manifest.layers[0].media_type = MEDIA_TYPE_BOOTER.to_owned();
    assert!(manifest.validate(false).is_err());
}

#[test]
fn manifest_validate_rejects_a_tree_layer_on_a_booter() {
    let mut manifest = tree_manifest();
    manifest.annotations = None;
    manifest.layers[0].media_type = MEDIA_TYPE_LAYER.to_owned();
    assert!(manifest.validate(true).is_err());
}

#[test]
fn manifest_validate_rejects_a_booter_with_two_layers() {
    let mut manifest = tree_manifest();
    manifest.annotations = None;
    manifest.layers = vec![
        descriptor(MEDIA_TYPE_BOOTER, &digest("a"), 1),
        descriptor(MEDIA_TYPE_BOOTER, &digest("b"), 1),
    ];
    assert!(manifest.validate(true).is_err());
}

#[test]
fn index_finds_a_tag_and_a_digest() {
    let mut index = Index::new();
    let mut annotations = BTreeMap::new();
    annotations.insert(ANNOTATION_REF_NAME.to_owned(), "demo:latest".to_owned());
    index.add(Descriptor {
        annotations: Some(annotations),
        ..descriptor(MEDIA_TYPE_MANIFEST, &digest("manifest"), 10)
    });
    assert!(index.find_ref("demo:latest").is_some());
    assert_eq!(index.find_digest(&digest("manifest")).len(), 1);
}

#[test]
fn index_remove_ref_keeps_other_tags() {
    let mut index = Index::new();
    for tag in ["a:latest", "b:latest"] {
        let mut annotations = BTreeMap::new();
        annotations.insert(ANNOTATION_REF_NAME.to_owned(), tag.to_owned());
        index.add(Descriptor {
            annotations: Some(annotations),
            ..descriptor(MEDIA_TYPE_MANIFEST, &digest(tag), 10)
        });
    }
    assert!(index.remove_ref("a:latest").is_some());
    assert!(index.find_ref("b:latest").is_some());
    assert!(index.find_ref("a:latest").is_none());
}

#[test]
fn oci_layout_has_version_1_0_0() {
    assert_eq!(OciLayout::new().image_layout_version, OCI_LAYOUT_VERSION);
}
