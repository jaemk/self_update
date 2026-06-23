/*! Semver version checks

The following functions compare two semver compatible version strings.
*/
use crate::errors::*;
use semver::Version;

/// Check if a version is greater than the current
pub fn bump_is_greater(current: &str, other: &str) -> Result<bool> {
    Ok(Version::parse(other)? > Version::parse(current)?)
}

/// Total-order comparison of two semver strings.
///
/// Parses each version once and returns their real semver [`Ordering`](std::cmp::Ordering) — so two
/// equal versions compare [`Equal`](std::cmp::Ordering::Equal) (unlike the boolean
/// [`bump_is_greater`], which collapses "equal" and "less" into `false`). An unparseable version
/// surfaces as an `Err`; callers that want a total order over a mixed list use the release
/// comparator built on this, which orders unparseable entries deterministically-last.
pub fn cmp_versions(a: &str, b: &str) -> Result<std::cmp::Ordering> {
    Ok(Version::parse(a)?.cmp(&Version::parse(b)?))
}

/// Newest-first (descending) total order over two version strings, ordering an unparseable version
/// deterministically **last**. Built on [`cmp_versions`]: a parseable pair compares by reversed
/// semver order (so the larger version sorts first); if exactly one side is unparseable it sorts
/// after the parseable one; two unparseable versions compare `Equal` (a stable no-op).
///
/// This is the shared release comparator the selection paths use (`choose_latest_release`,
/// `s3::sort_newer`/`pick_latest`), so they all agree on "newest" regardless of input order and
/// never panic on a junk version. The pre-filters those paths apply (`bump_is_greater(...)
/// .unwrap_or(false)`) already drop unparseable versions before this runs in the sort case, so the
/// unparseable handling here only matters for the `max_by` path, where it keeps a parseable release
/// winning over a junk one.
pub fn cmp_releases_newest_first(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (Version::parse(a), Version::parse(b)) {
        (Ok(a), Ok(b)) => b.cmp(&a),
        (Ok(_), Err(_)) => Ordering::Less,
        (Err(_), Ok(_)) => Ordering::Greater,
        (Err(_), Err(_)) => Ordering::Equal,
    }
}

/// Check if a new version is compatible with the current
pub fn bump_is_compatible(current: &str, other: &str) -> Result<bool> {
    let current = Version::parse(current)?;
    let other = Version::parse(other)?;
    Ok(if !current.pre.is_empty() {
        current.major == other.major
            && ((other.minor >= current.minor)
                || (current.minor == other.minor && other.patch >= current.patch))
    } else if other.major == 0 && current.major == 0 {
        current.minor == other.minor && other.patch > current.patch && other.pre.is_empty()
    } else if other.major > 0 {
        current.major == other.major
            && ((other.minor > current.minor)
                || (current.minor == other.minor && other.patch > current.patch))
            && other.pre.is_empty()
    } else {
        false
    })
}

/// Check if a new version is a major bump
pub fn bump_is_major(current: &str, other: &str) -> Result<bool> {
    let current = Version::parse(current)?;
    let other = Version::parse(other)?;
    Ok(other.major > current.major)
}

/// Check if a new version is a minor bump
pub fn bump_is_minor(current: &str, other: &str) -> Result<bool> {
    let current = Version::parse(current)?;
    let other = Version::parse(other)?;
    Ok(current.major == other.major && other.minor > current.minor)
}

/// Check if a new version is a patch bump
pub fn bump_is_patch(current: &str, other: &str) -> Result<bool> {
    let current = Version::parse(current)?;
    let other = Version::parse(other)?;
    Ok(current.major == other.major && current.minor == other.minor && other.patch > current.patch)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_bump_greater() {
        assert!(bump_is_greater("1.2.0", "1.2.3").unwrap());
        assert!(bump_is_greater("0.2.0", "1.2.3").unwrap());
        assert!(bump_is_greater("0.2.0", "0.2.3").unwrap());
    }

    #[test]
    fn test_bump_is_compatible() {
        assert!(!bump_is_compatible("1.2.0", "2.3.1").unwrap());
        assert!(!bump_is_compatible("0.2.0", "2.3.1").unwrap());
        assert!(!bump_is_compatible("1.2.3", "3.3.0").unwrap());
        assert!(!bump_is_compatible("1.2.3", "0.2.0").unwrap());
        assert!(!bump_is_compatible("0.2.0", "0.3.0").unwrap());
        assert!(!bump_is_compatible("0.3.0", "0.2.0").unwrap());
        assert!(!bump_is_compatible("1.2.3", "1.1.0").unwrap());
        assert!(!bump_is_compatible("2.0.0", "2.0.0-alpha.1").unwrap());
        assert!(!bump_is_compatible("1.2.3", "2.0.0-alpha.1").unwrap());
        assert!(!bump_is_compatible("2.0.0-alpha.1", "3.0.0").unwrap());

        assert!(bump_is_compatible("1.2.0", "1.2.3").unwrap());
        assert!(bump_is_compatible("0.2.0", "0.2.3").unwrap());
        assert!(bump_is_compatible("1.2.0", "1.3.3").unwrap());
        assert!(bump_is_compatible("2.0.0-alpha.0", "2.0.0-alpha.1").unwrap());
        assert!(bump_is_compatible("2.0.0-alpha.0", "2.0.0").unwrap());
        assert!(bump_is_compatible("2.0.0-alpha.0", "2.0.1").unwrap());
        assert!(bump_is_compatible("2.0.0-alpha.0", "2.1.0").unwrap());
    }

    #[test]
    fn test_bump_is_major() {
        assert!(bump_is_major("1.2.0", "2.3.1").unwrap());
        assert!(bump_is_major("0.2.0", "2.3.1").unwrap());
        assert!(bump_is_major("1.2.3", "3.3.0").unwrap());
        assert!(!bump_is_major("1.2.3", "1.2.0").unwrap());
        assert!(!bump_is_major("1.2.3", "0.2.0").unwrap());
    }

    #[test]
    fn test_bump_is_minor() {
        assert!(!bump_is_minor("1.2.0", "2.3.1").unwrap());
        assert!(!bump_is_minor("0.2.0", "2.3.1").unwrap());
        assert!(!bump_is_minor("1.2.3", "3.3.0").unwrap());
        assert!(bump_is_minor("1.2.3", "1.3.0").unwrap());
        assert!(bump_is_minor("0.2.3", "0.4.0").unwrap());
    }

    #[test]
    fn cmp_versions_returns_equal_for_equal_versions() {
        use std::cmp::Ordering;
        assert_eq!(cmp_versions("1.2.3", "1.2.3").unwrap(), Ordering::Equal);
        assert_eq!(cmp_versions("0.0.0", "0.0.0").unwrap(), Ordering::Equal);
        // Pre-release equal to itself.
        assert_eq!(
            cmp_versions("2.0.0-alpha.1", "2.0.0-alpha.1").unwrap(),
            Ordering::Equal
        );
    }

    #[test]
    fn cmp_versions_orders_and_is_antisymmetric() {
        use std::cmp::Ordering;
        assert_eq!(cmp_versions("1.2.3", "2.0.0").unwrap(), Ordering::Less);
        assert_eq!(cmp_versions("2.0.0", "1.2.3").unwrap(), Ordering::Greater);
        // Antisymmetry: swapping the args reverses the ordering for every parseable pair.
        for (a, b) in [
            ("1.0.0", "1.0.1"),
            ("0.9.0", "1.0.0"),
            ("2.0.0-alpha.1", "2.0.0"),
            ("1.2.3", "1.2.3"),
        ] {
            let ab = cmp_versions(a, b).unwrap();
            let ba = cmp_versions(b, a).unwrap();
            assert_eq!(
                ab,
                ba.reverse(),
                "cmp_versions({a}, {b}) must be the reverse of cmp({b}, {a})"
            );
        }
    }

    #[test]
    fn cmp_versions_errors_on_unparseable() {
        assert!(cmp_versions("not-a-version", "1.0.0").is_err());
        assert!(cmp_versions("1.0.0", "junk").is_err());
    }

    #[test]
    fn cmp_releases_newest_first_orders_newest_first_and_pushes_junk_last() {
        use std::cmp::Ordering;
        // Newest-first: the larger version sorts before (Less) the smaller.
        assert_eq!(cmp_releases_newest_first("2.0.0", "1.0.0"), Ordering::Less);
        assert_eq!(
            cmp_releases_newest_first("1.0.0", "2.0.0"),
            Ordering::Greater
        );
        assert_eq!(cmp_releases_newest_first("1.0.0", "1.0.0"), Ordering::Equal);

        // A parseable version always sorts before an unparseable one (junk last).
        assert_eq!(
            cmp_releases_newest_first("1.0.0", "junk"),
            Ordering::Less,
            "parseable sorts before junk"
        );
        assert_eq!(
            cmp_releases_newest_first("junk", "1.0.0"),
            Ordering::Greater,
            "junk sorts after parseable"
        );
        // Two unparseable versions are a stable no-op.
        assert_eq!(
            cmp_releases_newest_first("junk", "garbage"),
            Ordering::Equal
        );
    }

    #[test]
    fn cmp_releases_newest_first_sorts_a_list_descending_with_junk_last() {
        let mut versions = ["1.0.0", "junk", "2.1.0", "1.5.0", "garbage"];
        versions.sort_by(|a, b| cmp_releases_newest_first(a, b));
        // Parseable versions newest-first, then the two unparseable ones (in their relative order).
        assert_eq!(&versions[..3], &["2.1.0", "1.5.0", "1.0.0"]);
        assert!(Version::parse(versions[3]).is_err());
        assert!(Version::parse(versions[4]).is_err());
    }

    #[test]
    fn test_bump_is_patch() {
        assert!(!bump_is_patch("1.2.0", "2.3.1").unwrap());
        assert!(!bump_is_patch("0.2.0", "2.3.1").unwrap());
        assert!(!bump_is_patch("1.2.3", "3.3.0").unwrap());
        assert!(!bump_is_patch("1.2.3", "1.2.3").unwrap());
        assert!(bump_is_patch("1.2.0", "1.2.3").unwrap());
        assert!(bump_is_patch("0.2.3", "0.2.4").unwrap());
    }
}
