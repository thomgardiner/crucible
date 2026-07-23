//! The coverage floor: objective reachability. Parses an LCOV report (the format every
//! coverage tool can emit — cargo-llvm-cov, c8, tarpaulin, nyc) and reports, for the
//! files a change touched, which functions no test ever calls, which code paths
//! (branches) are never taken, and how many lines are never executed.
//!
//! Coverage is a floor, not a gate. It cannot prove a test asserts anything (that is
//! mutation's job in `harden`), so it is never a standalone metric to optimize. But a
//! zero-hit function is un-gameable proof it is not exercised at all, and a never-taken
//! branch is an untested code path (usually an error/else arm). Coverage answers "was it
//! run", mutation answers "was it checked".

use std::collections::{BTreeSet, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct FnCov {
    pub name: String,
    pub line: u32,
    pub hits: u64,
}

#[derive(Debug, Clone)]
pub struct FileCov {
    pub file: String,
    pub functions: Vec<FnCov>,
    pub uncovered_lines: Vec<u32>,
    // Lines with at least one branch arm that no test ever takes.
    pub untested_branches: Vec<u32>,
}

// Parse an LCOV report into per-file coverage. LCOV records: SF (source file), FN
// (function line + name), FNDA (function hit count + name), DA (line + hit count), BRDA
// (line, block, branch, taken), end_of_record. A function present as FN with no FNDA is
// zero hits; a BRDA whose taken is 0 or "-" is a branch arm no test took.
pub fn parse_lcov(text: &str) -> Vec<FileCov> {
    let mut files = vec![];
    let mut cur_file: Option<String> = None;
    let mut fn_lines: Vec<(String, u32)> = vec![];
    let mut fn_hits: Vec<(String, u64)> = vec![];
    let mut uncovered: Vec<u32> = vec![];
    let mut branches: BTreeSet<u32> = BTreeSet::new();

    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SF:") {
            cur_file = Some(rest.to_string());
            fn_lines.clear();
            fn_hits.clear();
            uncovered.clear();
            branches.clear();
        } else if let Some(rest) = line.strip_prefix("FN:") {
            if let Some((n, name)) = rest.split_once(',')
                && let Ok(n) = n.trim().parse::<u32>()
            {
                fn_lines.push((name.to_string(), n));
            }
        } else if let Some(rest) = line.strip_prefix("FNDA:") {
            if let Some((h, name)) = rest.split_once(',')
                && let Ok(h) = h.trim().parse::<u64>()
            {
                fn_hits.push((name.to_string(), h));
            }
        } else if let Some(rest) = line.strip_prefix("DA:") {
            if let Some((n, h)) = rest.split_once(',')
                && let (Ok(n), Ok(h)) = (n.trim().parse::<u32>(), h.trim().parse::<u64>())
                && h == 0
            {
                uncovered.push(n);
            }
        } else if let Some(rest) = line.strip_prefix("BRDA:") {
            // BRDA:<line>,<block>,<branch>,<taken>. taken is a count or "-" (not reached).
            let parts: Vec<&str> = rest.split(',').collect();
            if parts.len() == 4
                && let Ok(n) = parts[0].trim().parse::<u32>()
            {
                let taken = parts[3].trim();
                if taken == "-" || taken == "0" {
                    branches.insert(n);
                }
            }
        } else if line == "end_of_record"
            && let Some(file) = cur_file.take()
        {
            files.push(build_record(
                file,
                &fn_lines,
                &fn_hits,
                &mut uncovered,
                &mut branches,
            ));
            fn_lines.clear();
            fn_hits.clear();
        }
    }
    // A report whose last file has no trailing `end_of_record` must not be dropped.
    if let Some(file) = cur_file.take() {
        files.push(build_record(
            file,
            &fn_lines,
            &fn_hits,
            &mut uncovered,
            &mut branches,
        ));
    }
    files
}

fn build_record(
    file: String,
    fn_lines: &[(String, u32)],
    fn_hits: &[(String, u64)],
    uncovered: &mut Vec<u32>,
    branches: &mut BTreeSet<u32>,
) -> FileCov {
    let functions = fn_lines
        .iter()
        .map(|(name, l)| FnCov {
            // Demangle Rust symbols so the report reads `ProxyLease::release` rather than
            // `_RNvMNtCs..5lease..`. `{:#}` drops the hash suffix; a non-mangled name passes
            // through unchanged.
            name: format!("{:#}", rustc_demangle::demangle(name)),
            line: *l,
            hits: fn_hits
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, h)| *h)
                .unwrap_or(0),
        })
        .collect();
    FileCov {
        file,
        functions,
        uncovered_lines: std::mem::take(uncovered),
        untested_branches: std::mem::take(branches).into_iter().collect(),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Untested {
    pub file: String,
    pub name: String,
    pub line: u32,
    pub high_risk: bool,
}

pub struct CoverReport {
    pub untested: Vec<Untested>,
    pub uncovered_line_count: usize,
    // (file, line) of code paths no test takes, in the changed files.
    pub untested_branch_lines: Vec<(String, u32)>,
    // Matched changed files whose records carry no FN: data at all. Per-file, not
    // aggregate (Codex round 4): one changed file with function evidence must not mask
    // another whose records have none — for that file "every changed function is
    // exercised" would be vacuously true, so the CLI must refuse to certify it.
    pub fn_less_matched: Vec<String>,
    // Changed files whose extension the LCOV report covers elsewhere but which match no
    // record at all — e.g. a brand-new source file no test ever compiles. Silently skipping
    // them would certify exactly the least-tested code.
    pub unmatched_changed: Vec<String>,
    pub verdict: String,
}

// An LCOV source path and a git-diff path rarely match verbatim (absolute vs relative),
// so match when one is a path-component suffix of the other. Matching on a component
// boundary stops `notfoo.rs` from matching a changed `foo.rs`. Windows LCOV emits `\`
// separators while git diff emits `/`, so both sides are normalized first (Codex
// round 3: `SF:C:\repo\src\foo.rs` never matched a changed `src/foo.rs`).
fn same_file(lcov_path: &str, changed: &str) -> bool {
    let l = lcov_path.replace('\\', "/");
    let c = changed.replace('\\', "/");
    l == c || ends_with_component(&l, &c) || ends_with_component(&c, &l)
}

fn ends_with_component(hay: &str, tail: &str) -> bool {
    if !hay.ends_with(tail) {
        return false;
    }
    let cut = hay.len() - tail.len();
    cut == 0 || hay.as_bytes()[cut - 1] == b'/'
}

/// Match a high-risk unit against a path by path components, never a raw
/// substring: `pay` must not match `payload.rs`. A unit is a whole path segment
/// (`payments`), a path prefix (`src/payments`), or the exact path. Shared by
/// the mutation (harden) and coverage (cover) risk scoping so both agree.
pub fn path_hits_unit(path: &str, unit: &str) -> bool {
    let unit = unit.trim_matches('/');
    if unit.is_empty() {
        return false;
    }
    let path = path.replace('\\', "/");
    if path == unit
        || path.split('/').any(|seg| seg == unit)
        || path.starts_with(&format!("{unit}/"))
        || path.contains(&format!("/{unit}/"))
        || path.ends_with(&format!("/{unit}"))
    {
        return true;
    }
    // A unit may name a module by its file stem: `pay` matches `pay.rs` but not
    // `payload.rs` (the reward-hack a bare `contains` allowed).
    path.rsplit('/')
        .next()
        .and_then(|file| file.split('.').next())
        .is_some_and(|stem| stem == unit)
}

fn is_high_risk(path: &str, high_risk: &[String]) -> bool {
    high_risk.iter().any(|u| path_hits_unit(path, u))
}

fn extension(path: &str) -> Option<&str> {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
}

/// Extensions we treat as code that coverage tools should see. Kept deliberately
/// broad; non-source paths (md, json, toml) are not forced into the floor.
fn is_source_path(path: &str) -> bool {
    matches!(
        extension(path).map(|e| e.to_ascii_lowercase()).as_deref(),
        Some(
            "rs" | "go" | "py" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "c" | "cc"
                | "cpp" | "cxx" | "h" | "hpp" | "java" | "kt" | "kts" | "swift" | "m" | "mm"
                | "rb" | "php" | "cs" | "fs" | "scala" | "clj" | "ex" | "exs" | "zig" | "nim"
        )
    )
}

// Restrict coverage to the changed files. A never-called function blocks (in a high-risk
// unit) or is advisory; never-taken branches and uncovered lines are reported as the
// weaker signals they are. A coverage tool emits a function once per codegen instance;
// after demangling they collapse, so dedup by (file, name).
pub fn cover(files: &[FileCov], changed: &HashSet<String>, high_risk: &[String]) -> CoverReport {
    let mut untested = vec![];
    let mut untested_branch_lines = vec![];
    let mut uncovered_line_count = 0;
    // Per-file function-record tally: a file can span several records (codegen units), so
    // FN data in any of them counts for that file.
    let mut fn_by_file: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let mut seen = HashSet::new();

    // Changed *source* files with no LCOV SF match are untested. The floor uses
    // source-extension identity, not "extensions already present in this report",
    // so a `.js`-only LCOV cannot hide a changed `.rs` file. Docs/config stay out.
    let mut unmatched_changed: Vec<String> = changed
        .iter()
        .filter(|c| is_source_path(c))
        .filter(|c| !files.iter().any(|f| same_file(&f.file, c)))
        .cloned()
        .collect();
    unmatched_changed.sort();

    for fc in files {
        if !changed.iter().any(|c| same_file(&fc.file, c)) {
            continue;
        }
        *fn_by_file.entry(fc.file.as_str()).or_insert(0) += fc.functions.len();
        uncovered_line_count += fc.uncovered_lines.len();
        for l in &fc.untested_branches {
            untested_branch_lines.push((fc.file.clone(), *l));
        }
        for f in &fc.functions {
            if f.hits == 0 && seen.insert((fc.file.clone(), f.name.clone())) {
                untested.push(Untested {
                    file: fc.file.clone(),
                    name: f.name.clone(),
                    line: f.line,
                    high_risk: is_high_risk(&fc.file, high_risk),
                });
            }
        }
    }
    untested.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    untested_branch_lines.sort();
    let verdict = if untested.iter().any(|u| u.high_risk) {
        "block"
    } else if untested.is_empty() {
        "pass"
    } else {
        "advisory"
    };
    let fn_less_matched: Vec<String> = fn_by_file
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(f, _)| f.to_string())
        .collect();
    CoverReport {
        untested,
        uncovered_line_count,
        untested_branch_lines,
        fn_less_matched,
        unmatched_changed,
        verdict: verdict.to_string(),
    }
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
