//! Checking for and installing a newer release.
//!
//! A release is a GitHub release with a Windows binary attached. The check reads
//! the latest release's tag, compares it to the running version, and reports what
//! it found; installing is a separate step the user asks for.
//!
//! The download is a whole replacement binary rather than a patch. It is the only
//! thing that works without a package manager, and the binary is small enough that
//! it does not need to be anything cleverer.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Where releases are published.
const REPO: &str = "tiramission/myterm";

/// The asset name a Windows release carries.
const ASSET: &str = "myterm.exe";

/// The version this build reports.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag, without its leading `v`.
    pub version: String,
    /// Where to fetch the binary from.
    pub download_url: String,
    /// The release page, for a user who wants to read before installing.
    pub page_url: String,
}

#[derive(Debug)]
pub enum UpdateError {
    /// No release has been published. Not a failure: a project that has never
    /// shipped looks exactly like this, and saying so would be noise.
    NoRelease,
    /// The network or the API refused.
    Network(String),
    /// The answer did not parse, or carried no usable asset.
    Malformed(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRelease => write!(f, "no release published yet"),
            Self::Network(message) => write!(f, "{message}"),
            Self::Malformed(message) => write!(f, "{message}"),
        }
    }
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

/// Asks GitHub for the latest release.
///
/// Blocking: the caller runs it off the UI thread. GitHub requires a
/// `User-Agent`, and answers an unauthenticated request for a public repository
/// without a token, so no credential is involved.
pub fn check() -> Result<Release, UpdateError> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let response = ureq::get(&url)
        .header("User-Agent", concat!("myterm/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call();

    let mut response = match response {
        Ok(response) => response,
        // GitHub answers 404 for a repository with no releases, which is the
        // normal state of a project that has not shipped yet.
        Err(ureq::Error::StatusCode(404)) => return Err(UpdateError::NoRelease),
        Err(error) => return Err(UpdateError::Network(error.to_string())),
    };

    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    let latest: LatestRelease = serde_json::from_str(&body)
        .map_err(|error| UpdateError::Malformed(format!("release JSON: {error}")))?;

    let asset = latest
        .assets
        .iter()
        .find(|asset| asset.name == ASSET)
        .ok_or_else(|| UpdateError::Malformed(format!("{ASSET} is not attached to the release")))?;

    Ok(Release {
        version: latest.tag_name.trim_start_matches('v').to_owned(),
        download_url: asset.browser_download_url.clone(),
        page_url: latest.html_url,
    })
}

/// Whether `candidate` is newer than `current`.
///
/// Compares dot-separated numbers, missing components counting as zero, so
/// `0.2` beats `0.1.9` and a build without a patch number still compares. A tag
/// that does not parse is not newer: refusing to update is the safe direction.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |text: &str| -> Option<Vec<u64>> {
        text.split('.')
            .map(|part| {
                // A pre-release suffix (`1.0.0-beta.2`) is not something this app
                // publishes, and comparing it numerically would be wrong, so the
                // whole version is rejected rather than half-understood.
                part.parse::<u64>().ok()
            })
            .collect()
    };

    let (Some(candidate), Some(current)) = (parse(candidate), parse(current)) else {
        return false;
    };

    for index in 0..candidate.len().max(current.len()) {
        let a = candidate.get(index).copied().unwrap_or(0);
        let b = current.get(index).copied().unwrap_or(0);
        if a != b {
            return a > b;
        }
    }
    false
}

/// Downloads the release binary next to the running one.
///
/// Written beside the executable rather than over it: the running binary is
/// locked on Windows, so the swap has to happen after this process exits. The
/// returned path is the staged file, and it is the caller's job to arrange the
/// replacement.
pub fn download(release: &Release) -> Result<PathBuf, UpdateError> {
    let exe = std::env::current_exe()
        .map_err(|error| UpdateError::Network(format!("current executable: {error}")))?;
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let staged = dir.join("myterm.new.exe");

    let mut response = ureq::get(&release.download_url)
        .header("User-Agent", concat!("myterm/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .read_to_end(&mut bytes)
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    // A redirect to an HTML error page would otherwise be saved as if it were a
    // program, and the swap would leave the user with something that cannot start.
    if !bytes.starts_with(b"MZ") {
        return Err(UpdateError::Malformed(
            "the download is not a Windows executable".to_owned(),
        ));
    }

    std::fs::write(&staged, &bytes)
        .map_err(|error| UpdateError::Network(format!("writing {}: {error}", staged.display())))?;

    Ok(staged)
}

/// Replaces the running binary with `staged` and restarts.
///
/// A running executable cannot be overwritten on Windows, so this hands the work
/// to a short-lived script: it waits for this process to exit, moves the new file
/// into place, and starts it. The app then quits.
pub fn install_and_restart(staged: &Path) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let script = std::env::temp_dir().join("myterm-update.cmd");

    // `timeout` rather than `ping` for the wait: it is present everywhere and does
    // not depend on ICMP being allowed.
    let body = format!(
        "@echo off\r\n\
         timeout /t 2 /nobreak >nul\r\n\
         move /y \"{staged}\" \"{exe}\" >nul\r\n\
         start \"\" \"{exe}\"\r\n\
         del \"%~f0\"\r\n",
        staged = staged.display(),
        exe = exe.display(),
    );
    std::fs::write(&script, body)?;

    std::process::Command::new("cmd")
        .arg("/c")
        .arg("start")
        .arg("")
        .arg("/min")
        .arg(&script)
        .spawn()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_version_is_newer() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("1.0.0", "0.99.99"));
    }

    #[test]
    fn the_same_version_is_not_newer() {
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1", "0.1.0"));
    }

    #[test]
    fn a_lower_version_is_not_newer() {
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn a_short_version_compares_as_zero() {
        assert!(is_newer("0.2", "0.1.9"));
        assert!(!is_newer("0.1", "0.1.9"));
    }

    #[test]
    fn an_unparseable_tag_is_never_newer() {
        // Refusing to update is the safe direction.
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("1.0.0-beta.2", "0.1.0"));
    }
}
