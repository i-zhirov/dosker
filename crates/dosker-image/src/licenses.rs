//! Built-in license entries for the FreeDOS base and the DOS agent.

use dosker_core::license::{LicenseBasis, LicenseEntry, LicenseStatus};

/// The SPDX identifier for the pinned FreeDOS distribution.
pub const FREEDOS_SPDX: &str = "GPL-2.0-only";

/// The location of the FreeDOS source.
pub const FREEDOS_SOURCE_URL: &str = "https://github.com/FDOS";

/// Returns the license entry for FreeDOS.
pub fn freedos_license() -> LicenseEntry {
    LicenseEntry {
        name: "FreeDOS".to_owned(),
        spdx: FREEDOS_SPDX.to_owned(),
        holder: Some("FreeDOS contributors".to_owned()),
        status: LicenseStatus::Redistributable,
        basis: Some(LicenseBasis::OpenSourceLicense),
        evidence: None,
        license_url: Some("https://www.gnu.org/licenses/old-licenses/gpl-2.0.html".to_owned()),
        covers: Vec::new(),
        source_url: Some(FREEDOS_SOURCE_URL.to_owned()),
        notice: Some("FreeDOS is distributed under the GNU General Public License.".to_owned()),
    }
}

/// Returns the license entry for the DOSKER agent.
pub fn agent_license() -> LicenseEntry {
    LicenseEntry {
        name: "DOSKER".to_owned(),
        spdx: "MIT OR Apache-2.0".to_owned(),
        holder: Some("DOSker contributors".to_owned()),
        status: LicenseStatus::Redistributable,
        basis: Some(LicenseBasis::OpenSourceLicense),
        evidence: None,
        license_url: Some("https://github.com/i-zhirov/dosker#license".to_owned()),
        covers: Vec::new(),
        source_url: None,
        notice: Some("DOSker is dual-licensed under MIT or Apache-2.0.".to_owned()),
    }
}

/// Returns the entries every FreeDOS base carries.
pub fn freedos_base_licenses() -> Vec<LicenseEntry> {
    vec![freedos_license(), agent_license()]
}
