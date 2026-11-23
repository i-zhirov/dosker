//! License entries declared by a Doskerfile and stored in an image config.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The distribution class of one bundled program.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseStatus {
    /// The license permits redistribution.
    Redistributable,
    /// Distribution relies on the registry's declared abandonware exception.
    Abandonware,
}

impl LicenseStatus {
    /// Parses the Doskerfile value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "redistributable" => Some(Self::Redistributable),
            "abandonware" => Some(Self::Abandonware),
            _ => None,
        }
    }

    /// Returns the Doskerfile value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Redistributable => "redistributable",
            Self::Abandonware => "abandonware",
        }
    }
}

/// The stated basis for a license status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LicenseBasis {
    /// A recognized open-source license.
    OpenSourceLicense,
    /// An official freeware release.
    FreewareRelease,
    /// Public domain.
    PublicDomain,
    /// An explicit statement from the rights holder.
    RightsHolderStatement,
    /// A good-faith preservation claim for abandonware.
    GoodFaith,
}

impl LicenseBasis {
    /// Parses the Doskerfile value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "open-source-license" => Some(Self::OpenSourceLicense),
            "freeware-release" => Some(Self::FreewareRelease),
            "public-domain" => Some(Self::PublicDomain),
            "rights-holder-statement" => Some(Self::RightsHolderStatement),
            "good-faith" => Some(Self::GoodFaith),
            _ => None,
        }
    }

    /// Returns the Doskerfile value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenSourceLicense => "open-source-license",
            Self::FreewareRelease => "freeware-release",
            Self::PublicDomain => "public-domain",
            Self::RightsHolderStatement => "rights-holder-statement",
            Self::GoodFaith => "good-faith",
        }
    }
}

/// A summary of all license entries in one image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseSummary {
    /// Every entry is redistributable.
    Redistributable,
    /// Every entry is abandonware.
    Abandonware,
    /// The entries mix both statuses.
    Mixed,
}

impl LicenseSummary {
    /// Returns the contract value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Redistributable => "redistributable",
            Self::Abandonware => "abandonware",
            Self::Mixed => "mixed",
        }
    }
}

/// One bundled program and its license terms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseEntry {
    /// The program token.
    pub name: String,
    /// An SPDX expression, or `NONE`.
    pub spdx: String,
    /// The copyright holder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder: Option<String>,
    /// The distribution class.
    pub status: LicenseStatus,
    /// The stated basis for the status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<LicenseBasis>,
    /// The evidence URL for a non-license basis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
    /// A URL to the license text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_url: Option<String>,
    /// The DOS path globs this entry owns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// A URL to the corresponding source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// A short notice shown to users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
}

impl LicenseEntry {
    /// Validates the fields and the conditional rules.
    pub fn validate(&self) -> Result<(), LicenseError> {
        if !valid_name(&self.name) {
            return Err(invalid("NAME must be a token of 1 to 64 characters"));
        }
        let spdx_text = self.spdx.trim();
        if spdx_text.is_empty() {
            return Err(invalid("SPDX must not be empty"));
        }
        if let Some(holder) = &self.holder
            && (holder.is_empty() || holder.chars().count() > 200)
        {
            return Err(invalid("HOLDER must be 1 to 200 characters"));
        }
        if let Some(notice) = &self.notice
            && notice.chars().count() > 2000
        {
            return Err(invalid("NOTICE must be at most 2000 characters"));
        }
        for cover in &self.covers {
            if cover.is_empty() || cover.chars().count() > 128 {
                return Err(invalid("each COVERS glob must be 1 to 128 characters"));
            }
        }
        for (key, value) in [
            ("EVIDENCE", &self.evidence),
            ("LICENSE_URL", &self.license_url),
            ("SOURCE_URL", &self.source_url),
        ] {
            if let Some(value) = value
                && value.trim().is_empty()
            {
                return Err(invalid(format!("{key} must not be empty")));
            }
        }

        let spdx_none = spdx_text == "NONE";
        if !spdx_none {
            spdx::Expression::parse_mode(spdx_text, spdx::ParseMode::STRICT)
                .map_err(|error| invalid(format!("SPDX is not valid: {error}")))?;
        }

        if spdx_text.contains("LicenseRef-") && self.license_url.is_none() {
            return Err(invalid(
                "LICENSE_URL is required for a LicenseRef expression",
            ));
        }

        match self.status {
            LicenseStatus::Abandonware => {
                if !spdx_none {
                    return Err(invalid("abandonware requires SPDX=NONE"));
                }
                if self.basis != Some(LicenseBasis::GoodFaith) {
                    return Err(invalid("abandonware requires BASIS=good-faith"));
                }
                if self.evidence.is_none() {
                    return Err(invalid("abandonware requires EVIDENCE"));
                }
            }
            LicenseStatus::Redistributable => {
                if self.basis == Some(LicenseBasis::GoodFaith) {
                    return Err(invalid("BASIS=good-faith is valid only with abandonware"));
                }
                if self.basis == Some(LicenseBasis::OpenSourceLicense) && spdx_none {
                    return Err(invalid(
                        "BASIS=open-source-license requires a real SPDX expression",
                    ));
                }
                if spdx_none {
                    let allowed = matches!(
                        self.basis,
                        Some(
                            LicenseBasis::FreewareRelease
                                | LicenseBasis::PublicDomain
                                | LicenseBasis::RightsHolderStatement
                        )
                    );
                    if !allowed {
                        return Err(invalid(
                            "SPDX=NONE requires a freeware, public-domain, or rights-holder BASIS",
                        ));
                    }
                    if self.evidence.is_none() {
                        return Err(invalid("SPDX=NONE requires EVIDENCE"));
                    }
                }
            }
        }

        if self.is_copyleft() && self.source_url.is_none() {
            return Err(invalid("SOURCE_URL is required for a copyleft expression"));
        }

        Ok(())
    }

    /// Returns whether the SPDX expression contains a copyleft license.
    pub fn is_copyleft(&self) -> bool {
        let spdx_text = self.spdx.trim();
        if spdx_text.is_empty() || spdx_text == "NONE" {
            return false;
        }
        let Ok(expression) = spdx::Expression::parse_mode(spdx_text, spdx::ParseMode::STRICT)
        else {
            return false;
        };
        expression
            .requirements()
            .filter_map(|requirement| requirement.req.license.id())
            .any(|id| id.is_copyleft())
    }
}

/// Returns the summary for a set of entries, or `None` when the set is empty.
pub fn license_summary(entries: &[LicenseEntry]) -> Option<LicenseSummary> {
    let first = entries.first()?;
    if entries.iter().any(|entry| entry.status != first.status) {
        return Some(LicenseSummary::Mixed);
    }
    Some(match first.status {
        LicenseStatus::Redistributable => LicenseSummary::Redistributable,
        LicenseStatus::Abandonware => LicenseSummary::Abandonware,
    })
}

/// Validates every entry and rejects a duplicate name.
pub fn validate_licenses(entries: &[LicenseEntry]) -> Result<(), LicenseError> {
    let mut names = BTreeSet::new();
    for entry in entries {
        entry.validate()?;
        if !names.insert(entry.name.as_str()) {
            return Err(invalid(format!("duplicate license name: {}", entry.name)));
        }
    }
    Ok(())
}

/// Adds an entry, keeps one identical copy, and rejects a conflicting name.
pub fn merge_license_entry(
    entries: &mut Vec<LicenseEntry>,
    entry: LicenseEntry,
) -> Result<(), LicenseError> {
    if let Some(existing) = entries.iter().find(|existing| existing.name == entry.name) {
        if existing == &entry {
            return Ok(());
        }
        return Err(invalid(format!("conflicting license name: {}", entry.name)));
    }
    entries.push(entry);
    Ok(())
}

fn valid_name(name: &str) -> bool {
    let count = name.chars().count();
    if count == 0 || count > 64 {
        return false;
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '+' | '-'))
}

fn invalid(reason: impl Into<String>) -> LicenseError {
    LicenseError::Invalid {
        reason: reason.into(),
    }
}

/// A failure while validating a license entry.
#[derive(Debug, thiserror::Error)]
pub enum LicenseError {
    /// The entry does not satisfy the license rules.
    #[error("{reason}")]
    Invalid { reason: String },
}
