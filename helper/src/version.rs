//! The helper's version: one source (`Cargo.toml`), used by `--version`, the Options dialog and
//! the update check.

/// The version `Cargo.toml` names; release tags are `v` + this.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// `op36-helper 0.2.0`, what `--version` prints.
pub fn version_line() -> String {
    format!("op36-helper {VERSION}")
}

/// `major.minor.patch` from `0.2.1` or `v0.2.1`; prereleases and shorter forms are not versions
/// the updater may target.
pub fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let mut parts = s.split('.');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some((a.parse().ok()?, b.parse().ok()?, c.parse().ok()?))
}

/// `true` when `candidate` is a strictly newer version than `current`; unparsable input is never newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_semver(candidate), parse_semver(current)) {
        (Some(c), Some(cur)) => c > cur,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_the_cargo_version() {
        assert_eq!(
            version_line(),
            format!("op36-helper {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn semver_parses_with_or_without_v() {
        assert_eq!(parse_semver("v0.2.1"), Some((0, 2, 1)));
        assert_eq!(parse_semver("0.2.1"), Some((0, 2, 1)));
        assert_eq!(parse_semver("0.2"), None);
        assert_eq!(
            parse_semver("0.2.1-rc1"),
            None,
            "prereleases are not update targets"
        );
    }

    #[test]
    fn newer_compares_numerically() {
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("garbage", "0.2.0"));
    }
}
