//! Self-update, the agwinterm way: a GitHub release check, a verified download and a tiny script
//! that swaps the binary after the helper has exited. Everything that decides is here and pure;
//! the HTTP and file work lives in `update_thread`.

use std::path::{Path, PathBuf};

/// The public repository releases come from (`yeroo/op36-helper`).
pub const API_URL: &str = "https://api.github.com/repos/yeroo/op36-helper/releases/latest";

/// Which kind of install is running; decides how an update applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    #[cfg_attr(not(windows), allow(dead_code))]
    /// Inno per-user install under `%LOCALAPPDATA%\Programs\OP36 Helper`: silent setup re-run.
    Installed,
    /// A bare exe anywhere else: rename-swap in place.
    Portable,
    /// scoop / chocolatey layout (or a read-only system path on Linux): the package manager owns
    /// the files.
    PackageManager,
    /// Linux: the release tarball (binary + firmware) applied in place by the helper itself.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    LinuxTarball,
}

impl Channel {
    pub fn asset_kind(self) -> &'static str {
        match self {
            Channel::Installed => "setup",
            Channel::Portable => "portable",
            Channel::PackageManager => "package",
            Channel::LinuxTarball => "Linux",
        }
    }

    /// The word the apply script expects.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn script_name(self) -> &'static str {
        match self {
            Channel::Installed => "installed",
            _ => "portable",
        }
    }
}

/// The channel of the running copy on this platform.
pub fn current_channel(exe: &Path) -> Channel {
    #[cfg(windows)]
    {
        detect_channel(exe, &dirs::data_local_dir().unwrap_or_default())
    }
    #[cfg(not(windows))]
    {
        let dir_writable = exe
            .parent()
            .map(|d| {
                let probe = d.join(".op36-helper-write-probe");
                let ok = std::fs::write(&probe, b"").is_ok();
                let _ = std::fs::remove_file(&probe);
                ok
            })
            .unwrap_or(false);
        detect_channel_linux(exe, dir_writable)
    }
}

/// Linux: a copy in a system location (or anywhere we cannot write) belongs to its package
/// manager; anything else updates itself from the tarball.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn detect_channel_linux(exe: &Path, dir_writable: bool) -> Channel {
    let system = ["/usr/", "/nix/", "/snap/", "/opt/", "/app/"]
        .iter()
        .any(|p| exe.starts_with(p));
    if system || !dir_writable {
        Channel::PackageManager
    } else {
        Channel::LinuxTarball
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
pub fn detect_channel(exe: &Path, local_app_data: &Path) -> Channel {
    let exe_lower = exe.to_string_lossy().to_ascii_lowercase();
    if exe_lower.contains("\\scoop\\apps\\")
        || exe_lower.contains("\\chocolatey\\")
        || exe_lower.contains("\\windowsapps\\")
    {
        return Channel::PackageManager;
    }
    let install_dir = local_app_data.join("Programs").join("OP36 Helper");
    if exe.starts_with(&install_dir) {
        Channel::Installed
    } else {
        Channel::Portable
    }
}

/// One downloadable file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    /// Hex SHA-256 from the API's `digest: "sha256:…"`; other algorithms count as absent.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tag without its `v`.
    pub version: String,
    pub assets: Vec<Asset>,
}

/// A GitHub `/releases/latest` document; `None` on any shape mismatch (an update never proceeds
/// on a guess).
pub fn parse_release(json: &str) -> Option<Release> {
    let doc: serde_json::Value = serde_json::from_str(json).ok()?;
    let tag = doc.get("tag_name")?.as_str()?;
    let version = tag.strip_prefix('v').unwrap_or(tag).to_string();
    crate::version::parse_semver(&version)?;
    let assets = doc
        .get("assets")?
        .as_array()?
        .iter()
        .filter_map(|a| {
            let name = a.get("name")?.as_str()?.to_string();
            let url = a.get("browser_download_url")?.as_str()?.to_string();
            let sha256 = a
                .get("digest")
                .and_then(|d| d.as_str())
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(str::to_string);
            Some(Asset { name, url, sha256 })
        })
        .collect();
    Some(Release { version, assets })
}

/// Linux release tarball: `op36-helper-linux-x86_64-<version>.tar.gz` holding `op36-helper`, the
/// firmware files, the udev rule, the desktop entry and `install.sh`.
pub const LINUX_ASSET_PREFIX: &str = "op36-helper-linux-x86_64-";

/// The asset a channel updates from, if the release carries one.
pub fn pick_asset(release: &Release, channel: Channel) -> Option<&Asset> {
    let prefix = match channel {
        Channel::Installed => "op36-helper-setup-",
        Channel::Portable => "op36-helper-portable-",
        Channel::PackageManager => return None,
        Channel::LinuxTarball => {
            return release
                .assets
                .iter()
                .find(|a| a.name.starts_with(LINUX_ASSET_PREFIX) && a.name.ends_with(".tar.gz"));
        }
    };
    release
        .assets
        .iter()
        .find(|a| a.name.starts_with(prefix) && a.name.ends_with(".exe"))
}

/// Why an asset must not be installed, if so.
/// The version to offer, or nothing: newer than `current` *and* carrying an asset for this channel
/// with a verifiable digest. A release that is published before its files are uploaded (as
/// action-gh-release does) is not an update yet.
pub fn available_update(release: &Release, channel: Channel, current: &str) -> Option<String> {
    if !crate::version::is_newer(&release.version, current) {
        return None;
    }
    let asset = pick_asset(release, channel)?;
    if refuse_reason(asset).is_some() {
        return None;
    }
    Some(release.version.clone())
}

pub fn refuse_reason(asset: &Asset) -> Option<&'static str> {
    match &asset.sha256 {
        Some(hex) if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) => None,
        Some(_) => Some("the release asset carries a malformed digest"),
        None => Some("the release asset carries no SHA-256 digest"),
    }
}

/// `true` only when the file hashes to `expected_hex` (case-insensitive); missing file → `false`.
pub fn verify_sha256(path: &Path, expected_hex: &str) -> bool {
    use sha2::Digest;
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    let actual = sha2::Sha256::digest(&bytes);
    let actual_hex: String = actual.iter().map(|b| format!("{b:02x}")).collect();
    actual_hex.eq_ignore_ascii_case(expected_hex)
}

/// The PowerShell helper that applies the update after this process exits.
#[cfg(any(windows, test))]
pub fn apply_script() -> &'static str {
    include_str!("../assets/apply-update.ps1")
}

/// Where payloads and the helper script are written.
pub fn updates_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_default()
        .join("op36-helper")
        .join("updates")
}

/// Linux: where the firmware bundle lives when the binary sits in `~/.local/bin` (the tarball's
/// `install.sh` and the updater put it here).
#[cfg(target_os = "linux")]
pub fn linux_firmware_home() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_default()
        .join("op36-helper")
}

/// Linux: unpack a verified release tarball, refresh the firmware bundle and swap the binary in
/// place (rename over the running file is atomic; this process keeps its old inode). Returns the
/// path to start afterwards.
#[cfg(target_os = "linux")]
pub fn apply_linux_tarball(tarball: &Path, exe: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = tarball.parent().ok_or("bad payload path")?.join("unpacked");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(tarball)
        .arg("-C")
        .arg(&dir)
        .arg("--no-same-owner")
        .status()
        .map_err(|e| format!("cannot run tar: {e}"))?;
    if !status.success() {
        return Err(format!("tar failed ({status})"));
    }
    // The tarball has one top-level folder; accept a flat one too.
    let root = std::fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.join("op36-helper").is_file())
        .unwrap_or_else(|| dir.clone());
    let new_bin = root.join("op36-helper");
    if !new_bin.is_file() {
        return Err("the tarball holds no op36-helper binary".into());
    }
    // Firmware next to the exe wins when present (portable layout), else the data dir.
    let exe_dir = exe.parent().ok_or("bad exe path")?;
    let fw_target = if exe_dir.join(crate::firmware::BUNDLE_DIR).is_dir() {
        exe_dir.join(crate::firmware::BUNDLE_DIR)
    } else {
        linux_firmware_home().join(crate::firmware::BUNDLE_DIR)
    };
    let fw_src = root.join(crate::firmware::BUNDLE_DIR);
    if fw_src.is_dir() {
        std::fs::create_dir_all(&fw_target).map_err(|e| e.to_string())?;
        for entry in std::fs::read_dir(&fw_src).map_err(|e| e.to_string())?.flatten() {
            if entry.path().is_file() {
                std::fs::copy(entry.path(), fw_target.join(entry.file_name()))
                    .map_err(|e| format!("firmware copy: {e}"))?;
            }
        }
    }
    let staged = exe.with_extension("new");
    std::fs::copy(&new_bin, &staged).map_err(|e| format!("cannot stage the new binary: {e}"))?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| e.to_string())?;
    std::fs::rename(&staged, exe).map_err(|e| format!("cannot replace the binary: {e}"))?;
    Ok(exe.to_path_buf())
}

/// Start-up housekeeping: the `.old` exe a portable swap leaves and old payloads. Best effort.
pub fn cleanup_leftovers(exe: &Path) {
    let mut old = exe.as_os_str().to_owned();
    old.push(".old");
    let _ = std::fs::remove_file(PathBuf::from(old));
    let _ = std::fs::remove_dir_all(updates_dir());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SAMPLE: &str = r#"{"tag_name":"v0.2.1","assets":[{"name":"op36-helper-setup-0.2.1.exe","browser_download_url":"https://x/setup.exe","digest":"sha256:ab12000000000000000000000000000000000000000000000000000000000000"},{"name":"op36-helper-portable-0.2.1.exe","browser_download_url":"https://x/portable.exe"}]}"#;

    #[test]
    fn release_parses_version_assets_and_digests() {
        let r = parse_release(SAMPLE).unwrap();
        assert_eq!(r.version, "0.2.1");
        assert_eq!(
            pick_asset(&r, Channel::Installed)
                .unwrap()
                .sha256
                .as_deref(),
            Some("ab12000000000000000000000000000000000000000000000000000000000000")
        );
        assert_eq!(pick_asset(&r, Channel::Portable).unwrap().sha256, None);
        assert!(pick_asset(&r, Channel::PackageManager).is_none());
    }

    #[test]
    fn malformed_or_empty_release_is_no_update() {
        assert!(parse_release(r#"{"message":"API rate limit exceeded"}"#).is_none());
        assert!(parse_release("not json").is_none());
        let r = parse_release(r#"{"tag_name":"v9.9.9","assets":[]}"#).unwrap();
        assert!(pick_asset(&r, Channel::Installed).is_none());
    }

    #[test]
    fn missing_digest_refuses_update() {
        let r = parse_release(SAMPLE).unwrap();
        let portable = pick_asset(&r, Channel::Portable).unwrap();
        assert!(
            portable.sha256.is_none(),
            "the worker must refuse this asset"
        );
        assert_eq!(
            refuse_reason(portable),
            Some("the release asset carries no SHA-256 digest")
        );
        assert_eq!(
            refuse_reason(pick_asset(&r, Channel::Installed).unwrap()),
            None
        );
    }

    #[cfg(windows)] // Windows path semantics (`C:\...` prefixes)
    #[test]
    fn channel_detection_from_exe_path() {
        let lad = Path::new(r"C:\Users\b\AppData\Local");
        assert_eq!(
            detect_channel(
                Path::new(r"C:\Users\b\AppData\Local\Programs\OP36 Helper\op36-helper.exe"),
                lad
            ),
            Channel::Installed
        );
        assert_eq!(
            detect_channel(
                Path::new(r"C:\Users\b\scoop\apps\op36-helper\current\op36-helper.exe"),
                lad
            ),
            Channel::PackageManager
        );
        assert_eq!(
            detect_channel(Path::new(r"D:\tools\op36-helper.exe"), lad),
            Channel::Portable
        );
    }

    #[test]
    fn sha256_verification_fails_closed() {
        let f = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(f.path(), b"abc").unwrap();
        assert!(verify_sha256(
            f.path(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
        assert!(
            verify_sha256(
                f.path(),
                "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD"
            ),
            "case-insensitive"
        );
        assert!(!verify_sha256(f.path(), "00"));
        assert!(!verify_sha256(Path::new(r"Z:\missing\file.exe"), "00"));
    }

    #[test]
    fn linux_channel_detection() {
        assert_eq!(
            detect_channel_linux(Path::new("/home/b/.local/bin/op36-helper"), true),
            Channel::LinuxTarball
        );
        assert_eq!(
            detect_channel_linux(Path::new("/usr/bin/op36-helper"), true),
            Channel::PackageManager
        );
        assert_eq!(
            detect_channel_linux(Path::new("/home/b/ro/op36-helper"), false),
            Channel::PackageManager
        );
    }

    #[test]
    fn linux_asset_is_the_tarball() {
        let r = parse_release(r#"{"tag_name":"v0.3.0","assets":[{"name":"op36-helper-portable-0.3.0.exe","browser_download_url":"https://x/p.exe"},{"name":"op36-helper-linux-x86_64-0.3.0.tar.gz","browser_download_url":"https://x/l.tgz","digest":"sha256:ab12000000000000000000000000000000000000000000000000000000000000"}]}"#).unwrap();
        assert_eq!(
            pick_asset(&r, Channel::LinuxTarball).map(|a| a.url.as_str()),
            Some("https://x/l.tgz")
        );
        assert_eq!(
            available_update(&r, Channel::LinuxTarball, "0.2.6").as_deref(),
            Some("0.3.0")
        );
    }

    #[test]
    fn apply_script_knows_both_channels() {
        let s = apply_script();
        assert!(s.contains("/VERYSILENT"));
        assert!(s.contains("Wait-Process"));
        assert!(s.contains("Move-Item"), "portable swap");
    }
}

#[cfg(test)]
mod availability_tests {
    use super::*;

    fn release(version: &str, assets: Vec<Asset>) -> Release {
        Release {
            version: version.into(),
            assets,
        }
    }
    fn asset(name: &str, sha256: Option<&str>) -> Asset {
        Asset {
            name: name.into(),
            url: format!("https://x/{name}"),
            sha256: sha256.map(str::to_string),
        }
    }
    const GOOD: &str = "ab96664a40f9b71594f52cdfc9c7b6e77d3a41c9b216135563b533eb828b75d3";

    #[test]
    fn newer_release_without_an_asset_for_this_channel_is_not_available() {
        // action-gh-release publishes the release before its files are uploaded; the check must stay
        // quiet in that gap instead of raising a balloon that "Update now" then cannot honour.
        assert_eq!(
            available_update(&release("9.9.9", vec![]), Channel::Installed, "0.2.1"),
            None
        );
        let portable_only = release(
            "9.9.9",
            vec![asset("op36-helper-portable-9.9.9.exe", Some(GOOD))],
        );
        assert_eq!(
            available_update(&portable_only, Channel::Installed, "0.2.1"),
            None
        );
        let no_digest = release("9.9.9", vec![asset("op36-helper-setup-9.9.9.exe", None)]);
        assert_eq!(
            available_update(&no_digest, Channel::Installed, "0.2.1"),
            None
        );
    }

    #[test]
    fn newer_release_with_a_verifiable_asset_is_available() {
        let r = release(
            "9.9.9",
            vec![asset("op36-helper-setup-9.9.9.exe", Some(GOOD))],
        );
        assert_eq!(
            available_update(&r, Channel::Installed, "0.2.1").as_deref(),
            Some("9.9.9")
        );
        assert_eq!(
            available_update(&r, Channel::Installed, "9.9.9"),
            None,
            "same version is not an update"
        );
    }

    #[test]
    fn apply_script_relaunches_the_old_helper_when_it_cannot_apply() {
        let s = apply_script();
        assert!(
            s.contains("-LiteralPath"),
            "paths with [ ] must not be treated as wildcards"
        );
        assert!(
            !s.contains("-Path $Exe") && !s.contains("-Path $Payload"),
            "no wildcard -Path on user paths"
        );
        assert_eq!(
            s.matches("exit 1").count(),
            1,
            "every failure goes through one Fail path that relaunches"
        );
        let fail_at = s.find("function Fail").expect("a Fail function");
        let relaunch_in_fail = s[fail_at..]
            .find("Start-Process")
            .expect("Fail relaunches the helper");
        assert!(relaunch_in_fail < s[fail_at..].find("exit 1").unwrap());
        assert!(
            s.contains("ExitCode"),
            "the silent setup's exit code is checked"
        );
    }
}
