//! Tag names read as versions: semver (`v1.4.0`, `1.4.0`) and calendar
//! versions with a major (`v1-2026.10.05-1`). Nothing may come before either.
//!
//! Only a tag is ever read as a version — a branch called `v2.0.0` is a
//! branch — and two versions are compared only when they are of the same
//! kind. Everything else is compared by position in the graph, which is the
//! resolver's business, not this module's.

use std::cmp::Ordering;
use std::fmt;

/// A tag that reads as a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub scheme: Scheme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scheme {
    Semver(Semver),
    Calver(Calver),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Semver {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pre: Vec<Ident>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    Num(u64),
    Alpha(String),
}

/// `v<major>-YYYY.MM[.DD|.MICRO][.N]` and an optional modifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calver {
    major: u64,
    parts: Vec<u64>,
    modifier: Modifier,
}

/// What follows a calendar version's `-`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Modifier {
    /// `-rc1`, `-dev`: a pre-release, before the bare date.
    Pre(String),
    None,
    /// `-2`: the second release that day, after the bare date.
    Post(u64),
}

/// Which checkouts of one repository can be shared: the major — for semver
/// `0.x` the minor, as Cargo has it. A branch or SHA with no version tag
/// behind it is in `Any`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Class {
    Any,
    Zero(u64),
    Major(u64),
}

impl Class {
    /// What an extra checkout of this class gets after its name:
    /// `mylib_v2`, `mylib_v0.4`.
    pub fn suffix(self) -> String {
        match self {
            Class::Any => String::new(),
            Class::Zero(minor) => format!("_v0.{}", minor),
            Class::Major(major) => format!("_v{}", major),
        }
    }

    /// As `status --why` and the JSON output name it.
    pub fn describe(self) -> String {
        match self {
            Class::Any => "any".to_string(),
            Class::Zero(minor) => format!("0.{}", minor),
            Class::Major(major) => major.to_string(),
        }
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.describe())
    }
}

/// The years a calendar version may start with. A tag whose first number is
/// one of them and has no `v<major>-` is not a version at all, rather than
/// semver major 2026 — a project would need to reach major 1970 to mean it.
const YEARS: std::ops::RangeInclusive<u64> = 1970..=2199;

/// Read `tag` as a version, if it is one: `v<major>-` and a calendar version,
/// or an optional `v` and semver.
pub fn parse(tag: &str) -> Option<Version> {
    let rest = tag.strip_prefix('v').unwrap_or(tag);
    if let Some((major, calendar)) = rest.split_once('-') {
        if tag.starts_with('v') && is_major(major) && is_calendar(calendar) {
            let calver = parse_calver(number(major)?, calendar)?;
            return Some(Version {
                scheme: Scheme::Calver(calver),
            });
        }
    }
    if is_calendar(rest) {
        return None;
    }
    Some(Version {
        scheme: Scheme::Semver(parse_semver(rest)?),
    })
}

/// A calendar version's major: a number from 1, with no leading zero.
fn is_major(text: &str) -> bool {
    number(text).is_some_and(|n| n >= 1) && !text.starts_with('0')
}

/// Whether the version text starts with a four-digit year.
fn is_calendar(rest: &str) -> bool {
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.len() == 4
        && digits
            .parse::<u64>()
            .is_ok_and(|year| YEARS.contains(&year))
}

fn parse_calver(major: u64, text: &str) -> Option<Calver> {
    let (core, modifier) = match text.split_once('-') {
        Some((core, modifier)) => (core, Some(modifier)),
        None => (text, None),
    };
    let parts: Vec<u64> = core.split('.').map(number).collect::<Option<_>>()?;
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    let modifier = match modifier {
        None => Modifier::None,
        Some(m) if !m.is_empty() && m.chars().all(|c| c.is_ascii_digit()) => {
            Modifier::Post(m.parse().ok()?)
        }
        Some(m)
            if !m.is_empty()
                && m.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
                && m.chars().any(|c| c.is_ascii_alphabetic()) =>
        {
            Modifier::Pre(m.to_string())
        }
        Some(_) => return None,
    };
    Some(Calver {
        major,
        parts,
        modifier,
    })
}

fn parse_semver(text: &str) -> Option<Semver> {
    // Build metadata takes no part in precedence.
    let text = text.split_once('+').map_or(text, |(core, _)| core);
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (text, None),
    };
    let numbers: Vec<u64> = core.split('.').map(number).collect::<Option<_>>()?;
    let [major, minor, patch] = numbers[..] else {
        return None;
    };
    let pre = match pre {
        None => Vec::new(),
        Some(pre) => pre
            .split('.')
            .map(|ident| {
                if ident.is_empty() || !ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                {
                    None
                } else if let Some(n) = number(ident) {
                    Some(Ident::Num(n))
                } else {
                    Some(Ident::Alpha(ident.to_string()))
                }
            })
            .collect::<Option<_>>()?,
    };
    Some(Semver {
        major,
        minor,
        patch,
        pre,
    })
}

/// A run of ASCII digits, and nothing else.
fn number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl Version {
    pub fn class(&self) -> Class {
        match &self.scheme {
            Scheme::Semver(v) if v.major == 0 => Class::Zero(v.minor),
            Scheme::Semver(v) => Class::Major(v.major),
            Scheme::Calver(v) => Class::Major(v.major),
        }
    }

    pub fn is_semver(&self) -> bool {
        matches!(self.scheme, Scheme::Semver(_))
    }

    /// A pre-release: semver's `-rc.1`, a calendar version's text modifier.
    pub fn is_prerelease(&self) -> bool {
        match &self.scheme {
            Scheme::Semver(v) => !v.pre.is_empty(),
            Scheme::Calver(v) => matches!(v.modifier, Modifier::Pre(_)),
        }
    }

    /// How `self` and `other` order, or `None` when they are not of one
    /// kind and so cannot be compared as versions at all.
    pub fn compare(&self, other: &Version) -> Option<Ordering> {
        match (&self.scheme, &other.scheme) {
            (Scheme::Semver(a), Scheme::Semver(b)) => Some(a.cmp(b)),
            (Scheme::Calver(a), Scheme::Calver(b)) => Some(a.cmp(b)),
            _ => None,
        }
    }
}

impl Ord for Semver {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
                // A pre-release comes before the release it leads up to.
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => self.pre.cmp(&other.pre),
            })
    }
}

impl PartialOrd for Semver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Ident::Num(a), Ident::Num(b)) => a.cmp(b),
            (Ident::Alpha(a), Ident::Alpha(b)) => a.cmp(b),
            // Semver: numeric identifiers sort before alphanumeric ones.
            (Ident::Num(_), Ident::Alpha(_)) => Ordering::Less,
            (Ident::Alpha(_), Ident::Num(_)) => Ordering::Greater,
        }
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Calver {
    fn cmp(&self, other: &Self) -> Ordering {
        let width = self.parts.len().max(other.parts.len());
        let part = |parts: &[u64], i: usize| parts.get(i).copied().unwrap_or(0);
        let date = (0..width)
            .map(|i| part(&self.parts, i).cmp(&part(&other.parts, i)))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal);
        self.major
            .cmp(&other.major)
            .then(date)
            .then_with(|| self.modifier.cmp(&other.modifier))
    }
}

impl PartialOrd for Calver {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Modifier {
    fn cmp(&self, other: &Self) -> Ordering {
        let rank = |m: &Modifier| match m {
            Modifier::Pre(_) => 0,
            Modifier::None => 1,
            Modifier::Post(_) => 2,
        };
        match (self, other) {
            (Modifier::Pre(a), Modifier::Pre(b)) => natural(a, b),
            (Modifier::Post(a), Modifier::Post(b)) => a.cmp(b),
            _ => rank(self).cmp(&rank(other)),
        }
    }
}

impl PartialOrd for Modifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Natural order: runs of digits compare as numbers, everything else as
/// text, so `rc2` comes before `rc10`.
fn natural(a: &str, b: &str) -> Ordering {
    let (chunks_a, chunks_b) = (chunks(a), chunks(b));
    for (x, y) in chunks_a.iter().zip(&chunks_b) {
        let order = match (number(x), number(y)) {
            (Some(x), Some(y)) => x.cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => x.cmp(y),
        };
        if order.is_ne() {
            return order;
        }
    }
    chunks_a.len().cmp(&chunks_b.len())
}

fn chunks(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = text.as_bytes();
    for i in 1..=bytes.len() {
        if i == bytes.len() || bytes[i].is_ascii_digit() != bytes[i - 1].is_ascii_digit() {
            out.push(&text[start..i]);
            start = i;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(a: &str, b: &str) -> Option<Ordering> {
        parse(a).unwrap().compare(&parse(b).unwrap())
    }

    #[test]
    fn semver_with_or_without_v_is_the_same_version() {
        assert_eq!(order("v1.2.3", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(order("v1.2.4", "1.2.3"), Some(Ordering::Greater));
        assert_eq!(order("1.10.0", "v1.9.9"), Some(Ordering::Greater));
    }

    #[test]
    fn semver_precedence() {
        assert_eq!(order("1.0.0-alpha", "1.0.0"), Some(Ordering::Less));
        assert_eq!(
            order("1.0.0-alpha.1", "1.0.0-alpha"),
            Some(Ordering::Greater)
        );
        assert_eq!(
            order("1.0.0-alpha.beta", "1.0.0-alpha.1"),
            Some(Ordering::Greater)
        );
        assert_eq!(order("1.0.0-rc.11", "1.0.0-rc.2"), Some(Ordering::Greater));
        assert_eq!(order("1.0.0+build.5", "1.0.0"), Some(Ordering::Equal));
    }

    #[test]
    fn classes_follow_cargo_and_the_calendar_major() {
        assert_eq!(parse("v2.3.4").unwrap().class(), Class::Major(2));
        assert_eq!(parse("0.4.1").unwrap().class(), Class::Zero(4));
        assert_eq!(parse("v1-2026.10.01").unwrap().class(), Class::Major(1));
        assert_eq!(parse("v12-2026.10.01-3").unwrap().class(), Class::Major(12));
        assert_eq!(Class::Zero(4).suffix(), "_v0.4");
        assert_eq!(Class::Major(2).suffix(), "_v2");
        assert!(Class::Any < Class::Zero(1) && Class::Zero(9) < Class::Major(1));
    }

    #[test]
    fn a_calendar_version_needs_its_major() {
        let v = parse("v1-2026.01.01").unwrap();
        assert!(!v.is_semver());
        assert!(parse("v2-2026.10.01-2").is_some());
        assert!(parse("v1-2026.10").is_some());
        // A short year reads as semver.
        assert!(parse("26.10.0").unwrap().is_semver());
        // Not a year: an ordinary semver major.
        assert!(parse("3026.1.1").unwrap().is_semver());
    }

    #[test]
    fn calendar_modifiers_order_around_the_bare_date() {
        let run = [
            "v1-2026.10.01-dev",
            "v1-2026.10.01-rc2",
            "v1-2026.10.01-rc10",
            "v1-2026.10.01",
            "v1-2026.10.01-2",
            "v1-2026.10.01-11",
            "v1-2026.10.01-22",
            "v1-2026.10.02",
            "v2-2025.01.01",
        ];
        for pair in run.windows(2) {
            assert_eq!(order(pair[0], pair[1]), Some(Ordering::Less), "{:?}", pair);
        }
        assert_eq!(order("v1-2026.1.5", "v1-2026.01.05"), Some(Ordering::Equal));
    }

    #[test]
    fn semver_and_calendar_versions_never_compare() {
        assert_eq!(order("v1.4.0", "v1-2026.10.01"), None);
    }

    #[test]
    fn other_tags_are_not_versions() {
        for tag in [
            "main",
            "release",
            "1.2",
            "v1",
            "1.2.3.4",
            "2026",
            "x-1.2",
            "1.2.3-",
            "V1.2.3",
            "api-v1.4.0",
            "api-1.4.1",
            "release-1.4.0",
            "v2026.10.05",
            "2026.10.05-2",
            "1-2026.10.05",
            "v0-2026.10.05",
            "v01-2026.10.05",
            "v1-2026.1.1-",
            "api-v1-2026.10.05-1",
            "V1-2026.10.05",
        ] {
            assert!(parse(tag).is_none(), "{} parsed as {:?}", tag, parse(tag));
        }
    }
}
