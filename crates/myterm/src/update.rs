//! Checking for and installing a newer release.
//!
//! A release is a GitHub release with a Windows binary attached. The check reads
//! the latest release's tag, compares it to the running version, and reports what
//! it found; installing is a separate step the user asks for.
//!
//! The download is a whole replacement binary rather than a patch. It is the only
//! thing that works without a package manager, and the binary is small enough that
//! it does not need to be anything cleverer.

use std::path::{Path, PathBuf};

/// Where releases are published.
const REPO: &str = "tiramission/myterm";

/// The asset name a Windows release carries.
const ASSET: &str = "myterm.exe";

/// The largest download to accept. A ceiling is required because the body
/// reader will otherwise stream until the connection closes, and this is the
/// binary that replaces the running program.
const MAX_DOWNLOAD: u64 = 256 * 1024 * 1024;

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

/// Asks GitHub for the latest release.
///
/// Blocking: the caller runs it off the UI thread.
///
/// This reads the redirect that `releases/latest` answers with rather than the
/// API. The API is rate limited to sixty requests an hour per address without a
/// token, which a few restarts exhaust, and a terminal that cannot check for
/// updates because of its own checking is a bad trade. The redirect is not rate
/// limited, needs no credential, and carries the one thing needed: the tag.
pub fn check() -> Result<Release, UpdateError> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .build()
        .into();

    let response = agent
        .get(&url)
        .header("User-Agent", concat!("myterm/", env!("CARGO_PKG_VERSION")))
        .call();

    let location = match response {
        Ok(response) => response
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned),
        // A repository with no releases does not redirect anywhere, which is the
        // normal state of a project that has not shipped yet.
        Err(ureq::Error::StatusCode(404)) => return Err(UpdateError::NoRelease),
        Err(error) => return Err(UpdateError::Network(error.to_string())),
    };

    let Some(location) = location else {
        return Err(UpdateError::NoRelease);
    };

    // The location ends in the tag: `.../releases/tag/v0.1.0`.
    let tag = location
        .rsplit('/')
        .next()
        .filter(|tag| !tag.is_empty())
        .ok_or_else(|| UpdateError::Malformed(format!("unexpected redirect: {location}")))?;

    Ok(Release {
        version: tag.trim_start_matches('v').to_owned(),
        download_url: format!("https://github.com/{REPO}/releases/latest/download/{ASSET}"),
        page_url: location,
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
///
/// `progress` is called as bytes arrive, with how many have been written and
/// the total when the server declared one. A 22 MB download over a slow link
/// takes long enough that a badge saying only "正在更新…" looks stuck, and the
/// difference between slow and hung is the one thing the user needs to know.
pub fn download(
    release: &Release,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<PathBuf, UpdateError> {
    let exe = std::env::current_exe()
        .map_err(|error| UpdateError::Network(format!("current executable: {error}")))?;
    let dir = exe.parent().unwrap_or_else(|| Path::new("."));
    let staged = dir.join("myterm.new.exe");

    let mut response = ureq::get(&release.download_url)
        .header("User-Agent", concat!("myterm/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|error| UpdateError::Network(error.to_string()))?;

    // The declared length is advisory: it can be absent for a chunked response
    // and wrong for a compressed one, so the badge falls back to counting bytes
    // rather than dividing by a number it cannot trust.
    let total = response.body().content_length();

    let mut file = std::fs::File::create(&staged)
        .map_err(|error| UpdateError::Network(format!("create staged file: {error}")))?;

    // Streamed rather than read into memory: the buffer is one read at a time,
    // and the progress callback needs to fire as the bytes land instead of
    // after the whole file is in hand.
    let mut reader = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .reader();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut written = 0u64;
    // The first bytes decide whether this is a program at all, so they are kept
    // rather than re-read from disk afterwards.
    let mut head = Vec::with_capacity(2);

    loop {
        let read = std::io::Read::read(&mut reader, &mut buffer)
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        if read == 0 {
            break;
        }
        if head.len() < 2 {
            head.extend_from_slice(&buffer[..read.min(2 - head.len())]);
        }
        std::io::Write::write_all(&mut file, &buffer[..read])
            .map_err(|error| UpdateError::Network(format!("write staged file: {error}")))?;
        written += read as u64;
        progress(written, total);
    }

    drop(file);

    // A redirect to an HTML error page would otherwise be saved as if it were a
    // program, and the swap would leave the user with something that cannot start.
    if !head.starts_with(b"MZ") {
        let _ = std::fs::remove_file(&staged);
        return Err(UpdateError::Malformed(
            "the download is not a Windows executable".to_owned(),
        ));
    }

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

/// Bytes as whole megabytes, for a badge with no room for decimals.
pub fn megabytes(bytes: u64) -> String {
    format!("{} MB", bytes / (1024 * 1024))
}

#[cfg(test)]
mod progress_tests {
    use super::megabytes;

    #[test]
    fn megabytes_rounds_down_to_whole_units() {
        // The badge is read at a glance; a decimal would change too fast to
        // follow and the exact byte count is not what the user is asking.
        assert_eq!(megabytes(0), "0 MB");
        assert_eq!(megabytes(1024 * 1024), "1 MB");
        assert_eq!(megabytes(1024 * 1024 * 22 + 700_000), "22 MB");
    }
}
