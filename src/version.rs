//! Tag names read as versions: semver, calendar versions, and the tag streams
//! their prefixes name.
//!
//! Only a tag is ever read as a version — a branch called `v2.0.0` is a
//! branch — and two versions are compared only when they are in the same
//! stream and of the same kind. Everything else is compared by git history,
//! which is the resolver's business, not this module's.

use std::cmp::Ordering;
use std::fmt;

/// A tag that reads as a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// The tag stream: the text before the version, less a trailing `v`.
    /// Empty for `v1.2.3` and `1.2.3` alike.
    pub stream: String,
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

/// `YYYY.MM[.DD|.MICRO][.N]` and an optional modifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calver {
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

/// Which checkouts of one repository can be shared: the semver major, or for
/// `0.x` the minor, as Cargo has it. A calendar version, and a branch or SHA
/// with no semver tag behind it, are all in `Any`.
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

/// The years a calendar version may start with. Anything else in that
/// position is a semver major — a project would need to reach major 1970 to
/// be confused for one.
const YEARS: std::ops::RangeInclusive<u64> = 1970..=2199;

/// Read `tag` as a version, if it is one.
pub fn parse(tag: &str) -> Option<Version> {
    let start = tag.find(|c: char| c.is_ascii_digit())?;
    let (prefix, rest) = tag.split_at(start);
    let stream = stream_of(prefix);
    let scheme = if is_calendar(rest) {
        Scheme::Calver(parse_calver(rest)?)
    } else {
        Scheme::Semver(parse_semver(rest)?)
    };
    Some(Version { stream, scheme })
}

/// The stream a prefix names. A lone `v`, or one ending a prefix after a
/// separator (`api-v`), is spelling, not part of the stream's name.
fn stream_of(prefix: &str) -> String {
    match prefix.strip_suffix(['v', 'V']) {
        Some(before) if before.is_empty() || before.ends_with(['-', '_', '/', '.']) => {
            before.to_string()
        }
        _ => prefix.to_string(),
    }
}

/// Whether the version text starts with a four-digit year.
fn is_calendar(rest: &str) -> bool {
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.len() == 4
        && digits
            .parse::<u64>()
            .is_ok_and(|year| YEARS.contains(&year))
}

fn parse_calver(text: &str) -> Option<Calver> {
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
    Some(Calver { parts, modifier })
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
            Scheme::Calver(_) => Class::Any,
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

    /// How `self` and `other` order, or `None` when they are not versions of
    /// one stream and one kind and so cannot be compared as versions at all.
    pub fn compare(&self, other: &Version) -> Option<Ordering> {
        if self.stream != other.stream {
            return None;
        }
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
        (0..width)
            .map(|i| part(&self.parts, i).cmp(&part(&other.parts, i)))
            .find(|o| o.is_ne())
            .unwrap_or(Ordering::Equal)
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
    fn semver_with_or_without_v_is_one_stream() {
        assert_eq!(order("v1.2.3", "1.2.3"), Some(Ordering::Equal));
        assert_eq!(order("v1.2.4", "1.2.3"), Some(Ordering::Greater));
        assert_eq!(order("1.10.0", "v1.9.9"), Some(Ordering::Greater));
        assert_eq!(parse("v1.2.3").unwrap().stream, "");
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
    fn classes_follow_cargo() {
        assert_eq!(parse("v2.3.4").unwrap().class(), Class::Major(2));
        assert_eq!(parse("0.4.1").unwrap().class(), Class::Zero(4));
        assert_eq!(parse("v2026.10.01").unwrap().class(), Class::Any);
        assert_eq!(Class::Zero(4).suffix(), "_v0.4");
        assert_eq!(Class::Major(2).suffix(), "_v2");
        assert!(Class::Any < Class::Zero(1) && Class::Zero(9) < Class::Major(1));
    }

    #[test]
    fn a_year_first_tag_is_a_calendar_version() {
        let v = parse("v2026.01.01").unwrap();
        assert!(!v.is_semver());
        assert!(parse("2026.10.01-2").is_some());
        // A short year reads as semver.
        assert!(parse("26.10.0").unwrap().is_semver());
        // Not a year: an ordinary semver major.
        assert!(parse("3026.1.1").unwrap().is_semver());
    }

    #[test]
    fn calendar_modifiers_order_around_the_bare_date() {
        let run = [
            "2026.10.01-dev",
            "2026.10.01-rc2",
            "2026.10.01-rc10",
            "2026.10.01",
            "2026.10.01-2",
            "2026.10.01-11",
            "2026.10.01-22",
            "2026.10.02",
        ];
        for pair in run.windows(2) {
            assert_eq!(order(pair[0], pair[1]), Some(Ordering::Less), "{:?}", pair);
        }
        assert_eq!(order("2026.1.5", "2026.01.05"), Some(Ordering::Equal));
    }

    #[test]
    fn prefixes_name_streams() {
        assert_eq!(parse("api-v1.4.0").unwrap().stream, "api-");
        assert_eq!(parse("api-1.4.1").unwrap().stream, "api-");
        assert_eq!(parse("version-2026.09.30-2").unwrap().stream, "version-");
        assert_eq!(order("api-v1.4.0", "api-1.4.1"), Some(Ordering::Less));
        assert_eq!(order("api-v1.4.0", "v1.4.0"), None);
        assert_eq!(order("api-2026.10.01", "web-2026.10.02"), None);
        // Semver against a calendar version: not comparable as versions.
        assert_eq!(order("v1.4.0", "v2026.10.01"), None);
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
            "2026.1.1-",
        ] {
            assert!(parse(tag).is_none(), "{} parsed as {:?}", tag, parse(tag));
        }
    }
}
