pub(crate) fn is_newer(latest: &str, current: &str) -> Option<bool> {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => Some(l > c),
        _ => None,
    }
}

pub(crate) fn extract_version_from_latest_tag(latest_tag_name: &str) -> anyhow::Result<String> {
    latest_tag_name
        .strip_prefix("rust-v")
        .map(str::to_owned)
        .or_else(|| is_release_tag_version(latest_tag_name).then(|| latest_tag_name.to_string()))
        .ok_or_else(|| anyhow::anyhow!("Failed to parse latest tag name '{latest_tag_name}'"))
}

pub(crate) fn is_source_build_version(version: &str) -> bool {
    parse_version(version) == Some((0, 0, 0))
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let trimmed = v.trim();
    if let Some(version) = trimmed.strip_prefix("rust-v") {
        return parse_semver(version);
    }
    if let Some(version) = parse_semver(trimmed) {
        return Some(version);
    }
    parse_release_tag_version(trimmed)
}

fn parse_semver(v: &str) -> Option<(u64, u64, u64)> {
    let mut iter = v.split('.');
    let maj = iter.next()?.parse::<u64>().ok()?;
    let min = iter.next()?.parse::<u64>().ok()?;
    let pat = iter.next()?.parse::<u64>().ok()?;
    if iter.next().is_some() {
        return None;
    }
    Some((maj, min, pat))
}

fn parse_release_tag_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut iter = v.split('.');
    let year = iter.next()?.parse::<u64>().ok()?;
    let month = iter.next()?.parse::<u64>().ok()?;
    let day_and_suffix = iter.next()?;
    if iter.next().is_some() {
        return None;
    }

    let mut suffix_iter = day_and_suffix.split('-');
    let day = suffix_iter.next()?.parse::<u64>().ok()?;
    let sha = suffix_iter.next()?;
    if !(7..=10).contains(&sha.len()) || !sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }

    if let Some(extra) = suffix_iter.next()
        && extra != "dirty"
    {
        return None;
    }
    if suffix_iter.next().is_some() {
        return None;
    }

    Some((year, month, day))
}

fn is_release_tag_version(v: &str) -> bool {
    parse_release_tag_version(v).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn extracts_version_from_latest_tag() {
        assert_eq!(
            extract_version_from_latest_tag("rust-v1.5.0").expect("failed to parse version"),
            "1.5.0"
        );
    }

    #[test]
    fn latest_tag_without_prefix_is_invalid() {
        assert!(extract_version_from_latest_tag("v1.5.0").is_err());
    }

    #[test]
    fn extracts_release_tag_version() {
        assert_eq!(
            extract_version_from_latest_tag("2026.05.10-1f52616766")
                .expect("failed to parse release tag"),
            "2026.05.10-1f52616766"
        );
    }

    #[test]
    fn prerelease_version_is_not_considered_newer() {
        assert_eq!(is_newer("0.11.0-beta.1", "0.11.0"), None);
        assert_eq!(is_newer("1.0.0-rc.1", "1.0.0"), None);
    }

    #[test]
    fn release_tag_versions_compare_by_date() {
        assert_eq!(
            is_newer("2026.05.10-1f52616766", "2026.05.09-b8ac6d6b2c"),
            Some(true)
        );
        assert_eq!(
            is_newer("2026.05.09-b8ac6d6b2c", "2026.05.10-1f52616766"),
            Some(false)
        );
    }

    #[test]
    fn plain_semver_comparisons_work() {
        assert_eq!(is_newer("0.11.1", "0.11.0"), Some(true));
        assert_eq!(is_newer("0.11.0", "0.11.1"), Some(false));
        assert_eq!(is_newer("1.0.0", "0.9.9"), Some(true));
        assert_eq!(is_newer("0.9.9", "1.0.0"), Some(false));
    }

    #[test]
    fn source_build_version_is_not_checked() {
        assert!(is_source_build_version("0.0.0"));
        assert!(!is_source_build_version("0.1.0"));
    }

    #[test]
    fn whitespace_is_ignored() {
        assert_eq!(parse_version(" 1.2.3 \n"), Some((1, 2, 3)));
        assert_eq!(is_newer(" 1.2.3 ", "1.2.2"), Some(true));
    }
}
