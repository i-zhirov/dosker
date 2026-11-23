mod common;

use std::fs;
use std::path::Path;

use common::{CliRoot, stderr_text, stdout_text};

fn image_arg(image: &Path) -> &str {
    image.to_str().expect("the image path is utf-8")
}

#[test]
fn registry_validate_accepts_a_valid_fixture() {
    let root = CliRoot::new();
    let (_, image) = root.registry_fixture("i-zhirov", "i-zhirov");

    let output = root.run(&["registry", "validate", image_arg(&image)]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("validated images/i-zhirov/demo"),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[test]
fn registry_validate_rejects_an_uncovered_directory() {
    let root = CliRoot::new();
    let (_, image) = root.registry_fixture("i-zhirov", "i-zhirov");
    fs::write(
        image.join("Doskerfile"),
        "FROM base:latest\nCOPY APP C:\\APP\nLICENSE NAME=APP SPDX=MIT STATUS=redistributable BASIS=open-source-license COVERS=C:\\OTHER\\*\n",
    )
    .unwrap();

    let output = root.run(&["registry", "validate", image_arg(&image)]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("error:"),
        "{}",
        stderr_text(&output)
    );
}

#[test]
fn registry_validate_rejects_a_duplicate_tag() {
    let root = CliRoot::new();
    let (registry, image) = root.registry_fixture("i-zhirov", "i-zhirov");
    let second = registry.join("images/i-zhirov/two");
    fs::create_dir_all(&second).unwrap();
    fs::write(
        second.join("registry.toml"),
        "schema = 1\nversion = \"2.0.0\"\ntags = [\"latest\"]\ndescription = \"Two\"\n",
    )
    .unwrap();
    fs::write(second.join("Doskerfile"), "FROM base:latest\n").unwrap();

    let output = root.run(&["registry", "validate", image_arg(&image)]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("duplicate tag"),
        "{}",
        stderr_text(&output)
    );
}

#[test]
fn registry_validate_requires_a_path() {
    let root = CliRoot::new();

    let output = root.run(&["registry", "validate"]);

    assert_eq!(
        output.status.code(),
        Some(2),
        "stderr: {}",
        stderr_text(&output)
    );
}

#[test]
fn registry_validate_honours_the_registry_root() {
    let root = CliRoot::new();
    let (registry, image) = root.registry_fixture("i-zhirov", "i-zhirov");

    let output = root.run(&[
        "registry",
        "validate",
        image_arg(&image),
        "--registry-root",
        registry.to_str().expect("the registry path is utf-8"),
    ]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("validated images/i-zhirov/demo"),
        "stdout: {}",
        stdout_text(&output)
    );
}

fn final_license() -> dosker_core::LicenseEntry {
    dosker_core::LicenseEntry {
        name: "APP".to_owned(),
        spdx: "MIT".to_owned(),
        holder: None,
        status: dosker_core::LicenseStatus::Redistributable,
        basis: Some(dosker_core::LicenseBasis::OpenSourceLicense),
        evidence: None,
        license_url: None,
        covers: Vec::new(),
        source_url: None,
        notice: None,
    }
}

#[test]
fn registry_check_final_accepts_a_covered_image() {
    let root = CliRoot::new();
    root.store_final_image("final:latest", vec![final_license()], &[]);

    let output = root.run(&["registry", "check-final", "final:latest"]);

    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stdout_text(&output).contains("checked final:latest"),
        "stdout: {}",
        stdout_text(&output)
    );
}

#[test]
fn registry_check_final_rejects_an_uncovered_executable() {
    let root = CliRoot::new();
    root.store_final_image(
        "final:latest",
        vec![final_license()],
        &[("GAME.EXE", b"MZ")],
    );

    let output = root.run(&["registry", "check-final", "final:latest"]);

    assert_eq!(
        output.status.code(),
        Some(1),
        "stderr: {}",
        stderr_text(&output)
    );
    assert!(
        stderr_text(&output).contains("does not cover"),
        "{}",
        stderr_text(&output)
    );
}
