//! Updates (D-037): reading GitHub's "latest release" answer, comparing versions and checking a
//! downloaded installer against the release's `SHA256SUMS.txt`. The app does the networking.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// GitHub's API for the newest published (non-draft, non-prerelease) release.
pub const LATEST_RELEASE_URL: &str = "https://api.github.com/repos/moishebin/burrow/releases/latest";

/// A release newer than the running Burrow.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Release {
    pub version: String,
    /// The release's web page, for builds that can't install it themselves.
    pub page: String,
    /// The Windows setup's download URL and file name, if the release has one.
    pub setup: Option<Asset>,
    pub sums: Option<Asset>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// `1.2.3` (or `v1.2.3`, ignoring anything after a `-` or `+`) as numbers.
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    let (a, b, c) = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
    Some((a, b, c))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Reads GitHub's answer. `None` when it isn't newer than `current` (or isn't a real release).
pub fn newer_release(json: &str, current: &str) -> Result<Option<Release>, String> {
    let r: GhRelease = serde_json::from_str(json).map_err(|e| format!("unexpected answer from GitHub: {e}"))?;
    if r.draft || r.prerelease || !is_newer(&r.tag_name, current) {
        return Ok(None);
    }
    let version = r.tag_name.trim_start_matches('v').to_string();
    let find = |name: &str| {
        r.assets
            .iter()
            .find(|a| a.name == name)
            .map(|a| Asset { name: a.name.clone(), url: a.browser_download_url.clone() })
    };
    Ok(Some(Release {
        setup: find(&format!("burrow-{version}-windows-x64-setup.exe")),
        sums: find("SHA256SUMS.txt"),
        version,
        page: r.html_url,
    }))
}

/// The SHA-256 for `file` in a `sha256sum` listing ("<hex>  <name>" lines).
pub fn expected_sha256(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        (name.trim().trim_start_matches('*') == file && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks a download against the release's checksums. A release without a line for the file
/// fails: an installer that can't be checked isn't run.
pub fn verify(bytes: &[u8], sums: &str, file: &str) -> Result<(), String> {
    let want = expected_sha256(sums, file).ok_or_else(|| format!("SHA256SUMS.txt has no line for {file}"))?;
    let got = sha256_hex(bytes);
    if got == want { Ok(()) } else { Err(format!("{file} doesn't match its checksum")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{
        "tag_name": "v0.1.0", "html_url": "https://github.com/moishebin/burrow/releases/tag/v0.1.0",
        "draft": false, "prerelease": false,
        "assets": [
            {"name": "burrow-0.1.0-windows-x64.zip", "browser_download_url": "https://x/zip"},
            {"name": "burrow-0.1.0-windows-x64-setup.exe", "browser_download_url": "https://x/setup"},
            {"name": "SHA256SUMS.txt", "browser_download_url": "https://x/sums"}
        ]
    }"#;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.0.2"), Some((0, 0, 2)));
        assert_eq!(parse_version("1.10"), Some((1, 10, 0)));
        assert_eq!(parse_version("0.3.1-beta.2"), Some((0, 3, 1)));
        assert_eq!(parse_version("nightly"), None);
        assert!(is_newer("v0.0.10", "0.0.9"));
        assert!(!is_newer("v0.0.2", "0.0.2"));
        assert!(!is_newer("v0.0.1", "0.0.2"));
        assert!(!is_newer("garbage", "0.0.2"));
    }

    #[test]
    fn finds_newer_release_and_its_assets() {
        let r = newer_release(JSON, "0.0.2").unwrap().unwrap();
        assert_eq!(r.version, "0.1.0");
        assert_eq!(r.setup.unwrap().url, "https://x/setup");
        assert_eq!(r.sums.unwrap().name, "SHA256SUMS.txt");
        assert_eq!(newer_release(JSON, "0.1.0").unwrap(), None);
        let pre = JSON.replace(r#""prerelease": false"#, r#""prerelease": true"#);
        assert_eq!(newer_release(&pre, "0.0.2").unwrap(), None);
        assert!(newer_release("{}", "0.0.2").is_err());
    }

    #[test]
    fn checksums() {
        let data = b"burrow";
        let hash = sha256_hex(data);
        let sums = format!("{}  other.zip\n{hash}  burrow-0.1.0-windows-x64-setup.exe\n", "0".repeat(64));
        assert!(verify(data, &sums, "burrow-0.1.0-windows-x64-setup.exe").is_ok());
        assert!(verify(b"tampered", &sums, "burrow-0.1.0-windows-x64-setup.exe").is_err());
        assert!(verify(data, &sums, "missing.exe").is_err());
        // sha256sum's binary-mode marker.
        assert_eq!(expected_sha256(&format!("{hash} *a.exe"), "a.exe"), Some(hash));
    }
}
