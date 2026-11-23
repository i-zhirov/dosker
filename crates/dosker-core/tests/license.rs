use std::collections::BTreeSet;

use dosker_core::license::{
    LicenseBasis, LicenseEntry, LicenseStatus, LicenseSummary, license_summary, merge_license_entry,
};

fn entry(name: &str, spdx: &str, status: LicenseStatus) -> LicenseEntry {
    LicenseEntry {
        name: name.to_owned(),
        spdx: spdx.to_owned(),
        holder: None,
        status,
        basis: None,
        evidence: None,
        license_url: None,
        covers: Vec::new(),
        source_url: None,
        notice: None,
    }
}

fn open_source(name: &str, spdx: &str) -> LicenseEntry {
    let mut entry = entry(name, spdx, LicenseStatus::Redistributable);
    entry.basis = Some(LicenseBasis::OpenSourceLicense);
    entry
}

fn abandonware(name: &str) -> LicenseEntry {
    let mut entry = entry(name, "NONE", LicenseStatus::Abandonware);
    entry.basis = Some(LicenseBasis::GoodFaith);
    entry.evidence = Some("https://example.org/evidence".to_owned());
    entry
}

#[test]
fn an_open_source_entry_validates() {
    assert!(open_source("APP", "MIT").validate().is_ok());
}

#[test]
fn a_copyleft_expression_requires_a_source_url() {
    let mut entry = open_source("APP", "GPL-2.0-only");
    assert!(entry.validate().is_err());
    entry.source_url = Some("https://example.org/source".to_owned());
    assert!(entry.validate().is_ok());
}

#[test]
fn a_copyleft_expression_reports_copyleft() {
    assert!(open_source("APP", "GPL-2.0-only").is_copyleft());
    assert!(!open_source("APP", "MIT").is_copyleft());
    assert!(!entry("APP", "NONE", LicenseStatus::Abandonware).is_copyleft());
}

#[test]
fn a_contradictory_status_basis_and_spdx_is_rejected() {
    let mut entry = open_source("APP", "MIT");
    entry.status = LicenseStatus::Abandonware;
    assert!(entry.validate().is_err());
}

#[test]
fn an_unknown_spdx_id_is_rejected() {
    assert!(open_source("APP", "BOGUS-1.0").validate().is_err());
}

#[test]
fn a_license_reference_requires_a_license_url() {
    let mut entry = entry("APP", "LicenseRef-Custom", LicenseStatus::Redistributable);
    assert!(entry.validate().is_err());
    entry.license_url = Some("https://example.org/license".to_owned());
    assert!(entry.validate().is_ok());
}

#[test]
fn abandonware_requires_spdx_none_good_faith_and_evidence() {
    assert!(abandonware("GAME").validate().is_ok());

    let mut no_evidence = abandonware("GAME");
    no_evidence.evidence = None;
    assert!(no_evidence.validate().is_err());

    let mut real_spdx = abandonware("GAME");
    real_spdx.spdx = "MIT".to_owned();
    assert!(real_spdx.validate().is_err());

    let mut wrong_basis = abandonware("GAME");
    wrong_basis.basis = Some(LicenseBasis::OpenSourceLicense);
    assert!(wrong_basis.validate().is_err());
}

#[test]
fn an_open_source_basis_requires_a_real_expression() {
    let mut entry = entry("APP", "NONE", LicenseStatus::Redistributable);
    entry.basis = Some(LicenseBasis::OpenSourceLicense);
    entry.evidence = Some("https://example.org/evidence".to_owned());
    assert!(entry.validate().is_err());
}

#[test]
fn a_good_faith_basis_requires_abandonware() {
    let mut entry = entry("APP", "MIT", LicenseStatus::Redistributable);
    entry.basis = Some(LicenseBasis::GoodFaith);
    assert!(entry.validate().is_err());
}

#[test]
fn a_redistributable_none_entry_requires_a_free_basis_and_evidence() {
    let mut entry = entry("APP", "NONE", LicenseStatus::Redistributable);
    entry.basis = Some(LicenseBasis::PublicDomain);
    entry.evidence = Some("https://example.org/evidence".to_owned());
    assert!(entry.validate().is_ok());

    let mut no_evidence = entry.clone();
    no_evidence.evidence = None;
    assert!(no_evidence.validate().is_err());

    let mut open_source = entry.clone();
    open_source.basis = Some(LicenseBasis::OpenSourceLicense);
    assert!(open_source.validate().is_err());
}

#[test]
fn license_names_must_be_tokens() {
    assert!(open_source("GOOD.NAME+1", "MIT").validate().is_ok());
    assert!(open_source("BAD NAME", "MIT").validate().is_err());
    assert!(open_source("-BAD", "MIT").validate().is_err());
    assert!(open_source("", "MIT").validate().is_err());
}

#[test]
fn license_summary_reports_the_mixed_and_uniform_cases() {
    assert_eq!(license_summary(&[]), None);
    assert_eq!(
        license_summary(&[open_source("A", "MIT")]),
        Some(LicenseSummary::Redistributable)
    );
    assert_eq!(
        license_summary(&[abandonware("B")]),
        Some(LicenseSummary::Abandonware)
    );
    assert_eq!(
        license_summary(&[open_source("A", "MIT"), abandonware("B")]),
        Some(LicenseSummary::Mixed)
    );
}

#[test]
fn identical_license_entries_are_deduplicated() {
    let mut entries = vec![open_source("APP", "MIT")];
    merge_license_entry(&mut entries, open_source("APP", "MIT")).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn conflicting_license_entries_are_rejected() {
    let mut entries = vec![open_source("APP", "MIT")];
    let error = merge_license_entry(&mut entries, open_source("APP", "Apache-2.0")).unwrap_err();
    assert!(error.to_string().contains("APP"));
    assert_eq!(entries.len(), 1);
}

#[test]
fn license_entries_use_the_contract_json_shape() {
    let minimal = entry("APP", "MIT", LicenseStatus::Redistributable);
    let value = serde_json::to_value(&minimal).unwrap();
    let object = value.as_object().unwrap();
    let keys: BTreeSet<_> = object.keys().cloned().collect();
    assert_eq!(
        keys,
        ["name", "spdx", "status"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );

    let mut full = abandonware("GAME");
    full.holder = Some("Holder".to_owned());
    full.license_url = Some("https://example.org/license".to_owned());
    full.covers = vec!["C:\\GAME\\*".to_owned()];
    full.source_url = Some("https://example.org/source".to_owned());
    full.notice = Some("Notice".to_owned());
    let round_trip: LicenseEntry =
        serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
    assert_eq!(round_trip, full);
}

#[test]
fn license_status_and_basis_use_the_contract_values() {
    assert_eq!(
        serde_json::to_value(LicenseStatus::Redistributable).unwrap(),
        "redistributable"
    );
    assert_eq!(
        serde_json::to_value(LicenseStatus::Abandonware).unwrap(),
        "abandonware"
    );
    assert_eq!(
        serde_json::to_value(LicenseBasis::OpenSourceLicense).unwrap(),
        "open-source-license"
    );
    assert_eq!(
        serde_json::to_value(LicenseBasis::RightsHolderStatement).unwrap(),
        "rights-holder-statement"
    );
}
