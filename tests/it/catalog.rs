//! The test catalog: docs/test-catalog.md lists every test, and these tests
//! hold the suite to the naming rules in docs/testing.md.
//!
//! The catalog is generated from the sources. When a test is added, renamed or
//! removed, regenerate it and commit the result with the change:
//!
//! ```text
//! GITSCALE_UPDATE_CATALOG=1 cargo test --test it catalog::
//! ```
//!
//! A removed test then shows up as a removed line in the review.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const KINDS: [&str; 4] = ["normal", "edge", "error", "perf"];

struct Test {
    name: String,
    doc: String,
    ignored: Option<String>,
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Every `#[test]` function in `src`, with its doc paragraph and any
/// `#[ignore]` reason. Nested functions (indented) count too.
fn tests_in(src: &str) -> Vec<Test> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim() != "#[test]" {
            continue;
        }
        // Attributes after #[test], then the fn line.
        let mut k = i + 1;
        let mut ignored = None;
        while k < lines.len() && lines[k].trim_start().starts_with("#[") {
            let attr = lines[k].trim();
            if let Some(rest) = attr.strip_prefix("#[ignore") {
                let reason = rest
                    .trim_start_matches(" =")
                    .trim_start_matches('=')
                    .trim()
                    .trim_end_matches(']')
                    .trim()
                    .trim_matches('"');
                ignored = Some(if reason.is_empty() {
                    "ignored".to_string()
                } else {
                    reason.to_string()
                });
            }
            k += 1;
        }
        let Some(name) = lines
            .get(k)
            .and_then(|l| l.trim_start().strip_prefix("fn "))
            .and_then(|l| l.split('(').next())
        else {
            continue;
        };
        // Doc comment above #[test] (and above attributes before it).
        let mut j = i;
        while j > 0 && lines[j - 1].trim_start().starts_with("#[") {
            j -= 1;
        }
        let mut doc = Vec::new();
        while j > 0 && lines[j - 1].trim_start().starts_with("///") {
            j -= 1;
            doc.insert(0, lines[j].trim_start().trim_start_matches("///").trim());
        }
        let first_paragraph: Vec<&str> = doc.into_iter().take_while(|l| !l.is_empty()).collect();
        out.push(Test {
            name: name.to_string(),
            doc: first_paragraph.join(" "),
            ignored,
        });
    }
    out
}

/// `kind_NNN[x]_sentence` → (kind, number, variant, sentence).
fn parse_name(name: &str) -> Option<(&str, u32, Option<char>, &str)> {
    let (kind, rest) = name.split_once('_')?;
    if !KINDS.contains(&kind) {
        return None;
    }
    let (id, sentence) = rest.split_once('_')?;
    let digits: String = id.chars().take_while(|c| c.is_ascii_digit()).collect();
    let tail = &id[digits.len()..];
    if digits.len() != 3 || sentence.is_empty() {
        return None;
    }
    let variant = match tail.len() {
        0 => None,
        1 if tail.chars().all(|c| c.is_ascii_lowercase()) => tail.chars().next(),
        _ => return None,
    };
    Some((kind, digits.parse().ok()?, variant, sentence))
}

/// The feature modules of this binary and their tests, by module name.
fn integration_tests() -> BTreeMap<String, Vec<Test>> {
    let dir = repo().join("tests/it");
    let mut modules = BTreeMap::new();
    for entry in fs::read_dir(&dir).unwrap().flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "rs") && path.file_stem().unwrap() != "main" {
            let module = path.file_stem().unwrap().to_string_lossy().into_owned();
            modules.insert(module, tests_in(&fs::read_to_string(&path).unwrap()));
        }
    }
    modules
}

/// Unit tests, by source file relative to `src/`.
fn unit_tests() -> BTreeMap<String, Vec<Test>> {
    fn walk(dir: &Path, base: &Path, out: &mut BTreeMap<String, Vec<Test>>) {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, base, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let tests = tests_in(&fs::read_to_string(&path).unwrap());
                if !tests.is_empty() {
                    let rel = path.strip_prefix(base).unwrap().display().to_string();
                    out.insert(rel, tests);
                }
            }
        }
    }
    let mut out = BTreeMap::new();
    let src = repo().join("src");
    walk(&src, &src, &mut out);
    out
}

fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

fn render() -> String {
    let integration = integration_tests();
    let units = unit_tests();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total = 0;
    for tests in integration.values() {
        for t in tests {
            if let Some((kind, ..)) = parse_name(&t.name) {
                *counts.entry(kind).or_default() += 1;
            }
            total += 1;
        }
    }
    let unit_total: usize = units.values().map(Vec::len).sum();
    let ignored: usize = integration
        .values()
        .flatten()
        .filter(|t| t.ignored.is_some())
        .count();

    let mut md = String::new();
    md.push_str("# Test catalog\n\n");
    md.push_str(
        "Generated from the sources by `tests/it/catalog.rs` — do not edit by hand. \
         Regenerate with `GITSCALE_UPDATE_CATALOG=1 cargo test --test it catalog::`. \
         The naming rules are in [testing.md](testing.md).\n\n",
    );
    md.push_str(&format!(
        "{} integration tests ({}), {} of them ignored; {} unit tests.\n\n",
        total,
        KINDS
            .iter()
            .map(|k| format!("{} {}", counts.get(k).copied().unwrap_or(0), k))
            .collect::<Vec<_>>()
            .join(", "),
        ignored,
        unit_total
    ));
    md.push_str(
        "| Feature | normal | edge | error | perf | total |\n|---|---:|---:|---:|---:|---:|\n",
    );
    for (module, tests) in &integration {
        if tests.is_empty() {
            continue;
        }
        let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
        for t in tests {
            if let Some((kind, ..)) = parse_name(&t.name) {
                *by_kind.entry(kind).or_default() += 1;
            }
        }
        md.push_str(&format!(
            "| [{m}](#{m}) | {} | {} | {} | {} | {} |\n",
            by_kind.get("normal").copied().unwrap_or(0),
            by_kind.get("edge").copied().unwrap_or(0),
            by_kind.get("error").copied().unwrap_or(0),
            by_kind.get("perf").copied().unwrap_or(0),
            tests.len(),
            m = module
        ));
    }
    md.push_str("\n## Integration tests\n");
    for (module, tests) in &integration {
        if tests.is_empty() {
            continue;
        }
        md.push_str(&format!("\n### {}\n\n", module));
        md.push_str("| ID | Kind | Test | What it holds |\n|---|---|---|---|\n");
        let mut rows: Vec<_> = tests.iter().map(|t| (parse_name(&t.name), t)).collect();
        rows.sort_by_key(|(p, t)| (p.map(|(_, n, v, _)| (n, v)), t.name.clone()));
        for (parsed, t) in rows {
            let (id, kind) = match parsed {
                Some((kind, n, v, _)) => (
                    format!(
                        "{}-{:03}{}",
                        module,
                        n,
                        v.map(String::from).unwrap_or_default()
                    ),
                    kind,
                ),
                None => ("?".to_string(), "?"),
            };
            let mut what = cell(&t.doc);
            if let Some(reason) = &t.ignored {
                what = format!("**ignored: {}** {}", cell(reason), what);
            }
            md.push_str(&format!(
                "| {} | {} | `{}` | {} |\n",
                id,
                kind,
                t.name,
                what.trim()
            ));
        }
    }
    md.push_str("\n## Unit tests\n\nIn `#[cfg(test)]` modules beside the code, by source file.\n");
    for (file, tests) in &units {
        md.push_str(&format!("\n### src/{}\n\n", file));
        for t in tests {
            if t.doc.is_empty() {
                md.push_str(&format!("- `{}`\n", t.name));
            } else {
                md.push_str(&format!("- `{}` — {}\n", t.name, t.doc));
            }
        }
    }
    md
}

// ---------------------------------------------------------------------------
// Normal cases
// ---------------------------------------------------------------------------

/// Every integration test is named `<kind>_<id>[<variant>]_<sentence>`, its
/// id unique in its feature, and a variant letter only beside its siblings.
#[test]
fn normal_001_every_test_follows_the_naming_rules() {
    let mut problems = Vec::new();
    for (module, tests) in integration_tests() {
        let mut plain: BTreeSet<u32> = BTreeSet::new();
        let mut variants: BTreeMap<u32, BTreeSet<char>> = BTreeMap::new();
        for t in &tests {
            match parse_name(&t.name) {
                None => problems.push(format!(
                    "{}::{} does not follow the naming rule",
                    module, t.name
                )),
                Some((_, n, None, _)) => {
                    if !plain.insert(n) {
                        problems.push(format!("{}: id {:03} is used twice", module, n));
                    }
                }
                Some((_, n, Some(v), _)) => {
                    if !variants.entry(n).or_default().insert(v) {
                        problems.push(format!("{}: id {:03}{} is used twice", module, n, v));
                    }
                }
            }
        }
        for (n, letters) in &variants {
            if plain.contains(n) {
                problems.push(format!(
                    "{}: id {:03} is used both with and without a variant",
                    module, n
                ));
            }
            let expected: BTreeSet<char> = ('a'..).take(letters.len()).collect();
            if *letters != expected || letters.len() < 2 {
                problems.push(format!(
                    "{}: variants of {:03} must run a, b, … with at least two: {:?}",
                    module, n, letters
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// docs/test-catalog.md is what the sources say it is.
#[test]
fn normal_002_the_catalog_lists_every_test() {
    let path = repo().join("docs/test-catalog.md");
    let rendered = render();
    if std::env::var("GITSCALE_UPDATE_CATALOG").is_ok_and(|v| v == "1") {
        fs::write(&path, &rendered).unwrap();
        return;
    }
    let on_disk = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        on_disk == rendered,
        "docs/test-catalog.md is out of date: run\n  GITSCALE_UPDATE_CATALOG=1 cargo test --test it catalog::"
    );
}
