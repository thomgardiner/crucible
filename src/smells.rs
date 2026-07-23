//! Portable test-gaming detector. Screens Rust and TS/JS test files for the recurring
//! reward-hacking moves: skips without a reason, focused tests, assertion-free bodies,
//! and tautological asserts. A mechanical screen for the laziest hacks — it does not
//! replace the mutation keystone, it removes the cheapest ways to fake a green suite.

use crate::mask::{block_span, line_of, mask_comments_and_strings};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;
use walkdir::WalkDir;

static RUST_TEST_ATTR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#\[(?:tokio::|async_std::|actix_rt::)?test\b[^\]]*\]").unwrap());
// Any assert-family macro or helper in call position (`assert!`, `assert_eq!`,
// `prop_assert!`, `debug_assert!`, `assert_exact(`, `assert_matches!`), plus the panic
// family, `.expect(`/`.unwrap()`, and the `?` operator. Matching the whole `*assert*`
// family (not just `\bassert\b`) is what stops false positives on real code that asserts
// through a named helper or a prefixed macro; a genuinely hollow test still has none.
static RUST_ASSERTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\w*assert\w*\s*(?:!|\(|::)|\b(?:panic|todo|unimplemented|expect|unwrap|unwrap_err|prop_assume)\b|\?\s*;")
        .unwrap()
});
static RUST_IGNORE_NO_REASON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"#\[ignore\s*\]").unwrap());
static FN_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"fn\s+([a-zA-Z0-9_]+)").unwrap());
static ASSERT_TRUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bassert\s*!\s*\(\s*true\s*[,)]").unwrap());
static ASSERT_EQ_SELF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bassert_eq\s*!\s*\(\s*([a-zA-Z0-9_.]+)\s*,\s*([a-zA-Z0-9_.]+)\s*[,)]").unwrap()
});

static TS_SKIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:it|test|describe)(?:\.\w+)*\.(?:skip|todo)\b|\bx(?:it|test|describe)\b")
        .unwrap()
});
static TS_FOCUS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:it|test|describe)(?:\.\w+)*\.only\b|\bf(?:it|describe)\b").unwrap()
});
// A real test call passes a string/template name first (`it('x', …)`), which after
// masking is a preserved quote. Requiring it avoids matching a custom test-runner
// helper's own definition (`function test(name, fn) {…}`) as if it were a test.
static TS_TEST_CALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("\\b(?:it|test)\\s*\\(\\s*['\"`]").unwrap());
// Any assert/expect-family call (`expect(`, `assert(`, `assertForwarded(`,
// `expectTypeOf(`, `assertEquals(`) plus chai's `should`. Matching the whole family in
// call position stops false positives on tests that assert through a named helper.
// Assert/expect-family calls, node:assert member access, chai `should`, and a bare
// `throw` (a hand-rolled `if (!cond) throw new Error(...)` is a real assertion).
static TS_ASSERTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bassert\s*\.|\w*assert\w*\s*\(|\w*expect\w*\s*\(|\bshould\b|\bthrow\b").unwrap()
});
static TS_EXIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bprocess\.exit\s*\(\s*0\s*\)").unwrap());
static ARROW_OR_FN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"=>|\bfunction\b").unwrap());
static TS_EXPECT_SELF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"expect\s*\(\s*([a-zA-Z0-9_.]+)\s*\)\s*\.\s*(?:toBe|toEqual|toStrictEqual)\s*\(\s*([a-zA-Z0-9_.]+)\s*\)")
        .unwrap()
});
static TS_EXPECT_TRUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"expect\s*\(\s*true\s*\)\s*\.\s*toBe(?:Truthy)?\s*\(\s*(?:true\s*)?\)").unwrap()
});
static TS_TEST_FILE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\.(test|spec)\.(ts|tsx|js|jsx|mjs|cjs)$").unwrap());

// A success-return before the first assertion is the silent self-skip: the env-guard
// pattern (`if env::var("CI").is_err() { return; }`, or `return Ok(());` in a Result
// test) makes the test pass having tested nothing, with no skip marker in the report.
static RUST_BARE_RETURN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\breturn\s*(?:Ok\s*\(\s*\(\s*\)\s*\)\s*)?;").unwrap());
// `process::exit(0)` inside a test body ends the whole test binary green mid-run.
static RUST_EXIT_ZERO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bprocess::exit\s*\(\s*0\s*\)").unwrap());
static TS_TRY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btry\s*\{").unwrap());
// `catch {}` / `catch (e) {}` immediately after a try block: a failed assertion inside
// the try is swallowed and the test passes.
static EMPTY_CATCH_AT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*catch\s*(?:\([^)]*\))?\s*\{\s*\}").unwrap());
// `.catch(() => {})` / `.catch(e => undefined)`: a rejected assertion chain resolves
// clean. Expression bodies that discard the rejection count too (Codex round 5).
static TS_DOT_CATCH_EMPTY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\.catch\s*\(\s*(?:\(\s*[\w\s,]*\)|\w+)\s*=>\s*(?:\{\s*\}|undefined|null|false|0)\s*\)",
    )
    .unwrap()
});
static TS_DOT_THEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.then\s*\(").unwrap());

// Build a matcher for a repo's declared assertion helpers (names whose call means the
// test asserts, e.g. `recovers_after`, `assert_exact`). None when no helpers are declared.
pub fn helper_matcher(names: &[String]) -> Option<Regex> {
    if names.is_empty() {
        return None;
    }
    let alt = names
        .iter()
        .map(|n| regex::escape(n))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&format!(r"\b(?:{alt})\s*[(!.]")).ok()
}

fn asserts_via_helper(body: &str, helpers: Option<&Regex>) -> bool {
    helpers.map(|r| r.is_match(body)).unwrap_or(false)
}

#[cfg(test)]
pub fn inspect_rust(path: &str, raw: &str) -> Vec<String> {
    inspect_rust_with(path, raw, None)
}

fn inspect_rust_with(path: &str, raw: &str, helpers: Option<&Regex>) -> Vec<String> {
    let src = mask_comments_and_strings(raw);
    let mut failures = vec![];

    for m in RUST_IGNORE_NO_REASON.find_iter(&src) {
        failures.push(format!(
            "{path}:{}: #[ignore] without a reason — use #[ignore = \"why\"]",
            line_of(&src, m.start())
        ));
    }

    for m in RUST_TEST_ATTR.find_iter(&src) {
        // Find the fn that follows the attribute (skip over other attributes).
        let Some(rel) = src[m.start()..].find("fn ") else {
            continue;
        };
        let fn_idx = m.start() + rel;
        let Some(span) = block_span(&src, fn_idx) else {
            continue;
        };
        let body = &src[span.0..span.1];
        let name = FN_NAME
            .captures(&src[fn_idx..span.0])
            .and_then(|c| c.get(1))
            .map(|g| g.as_str().to_string())
            .unwrap_or_else(|| "?".into());
        // A #[should_panic] test asserts by panicking, so no explicit assertion is fine.
        // An #[ignore]d test does not run, so it is not a false green either.
        let attrs = &src[m.start()..fn_idx];
        let exempt = attrs.contains("should_panic")
            || attrs.contains("ignore")
            || asserts_via_helper(body, helpers);
        if !exempt && !RUST_ASSERTION.is_match(body) {
            failures.push(format!(
                "{path}:{}: test \"{name}\" has no assertion (no assert/panic/expect/unwrap/?) — it passes on any behavior",
                line_of(&src, fn_idx)
            ));
        }
        // The silent self-skip: a success-return (`return;` / `return Ok(());`) reachable
        // before the first assertion (an env/platform guard) passes the test having
        // tested nothing, with no skip marker in the report. Declared skips (#[ignore],
        // cfg_attr) are the honest form. Only guard-level returns count (brace depth ≤ 2:
        // the fn body or one `if` deep) — a return nested inside a loop is the poll-until
        // idiom, where returning IS the success path and the fall-through panics
        // unit_tests-style helpers that return Result without being a skip.
        // Every pre-assertion return is examined — an earlier loop-nested return must not
        // shadow a later guard-level one (Codex round 6).
        if !exempt
            && let Some(first_assert) = RUST_ASSERTION.find(body)
            && let Some(ret) = RUST_BARE_RETURN
                .find_iter(body)
                .take_while(|r| r.start() < first_assert.start())
                .find(|r| brace_depth(&body[..r.start()]) <= 2)
        {
            failures.push(format!(
                "{path}:{}: test \"{name}\" can return before its first assertion — a silent self-skip passes without testing; declare the skip with #[ignore = \"why\"] or #[cfg_attr(cond, ignore)]",
                line_of(&src, span.0 + ret.start())
            ));
        }
        // process::exit(0) inside a test ends the whole binary green mid-run — nothing
        // after it executes, and the harness reads success.
        if let Some(m) = RUST_EXIT_ZERO.find(body) {
            failures.push(format!(
                "{path}:{}: test \"{name}\" calls process::exit(0) — the test binary exits green mid-run and nothing after it executes",
                line_of(&src, span.0 + m.start())
            ));
        }
        failures.extend(rust_tautologies(path, &src, span, &name));
    }
    failures
}

// Net brace depth of a prefix that starts at a body's opening `{` (comments and strings
// already masked): 1 = directly in the fn body, 2 = one block deep (a guard `if`).
fn brace_depth(prefix: &str) -> i32 {
    prefix.bytes().fold(0i32, |d, b| match b {
        b'{' => d + 1,
        b'}' => d - 1,
        _ => d,
    })
}

fn rust_tautologies(path: &str, src: &str, span: (usize, usize), name: &str) -> Vec<String> {
    let mut out = vec![];
    let body = &src[span.0..span.1];
    if ASSERT_TRUE.is_match(body) {
        out.push(format!(
            "{path}:{}: test \"{name}\" asserts a literal true — tautological",
            line_of(src, span.0)
        ));
    }
    for c in ASSERT_EQ_SELF.captures_iter(body) {
        if c.get(1).map(|m| m.as_str()) == c.get(2).map(|m| m.as_str()) {
            out.push(format!(
                "{path}:{}: test \"{name}\" compares {} to itself — tautological",
                line_of(src, span.0),
                &c[1]
            ));
        }
    }
    out
}

// Extract the callback body of a test(...)/it(...) call to scan for assertions. Locates
// the callback (arrow or function) so an options-object argument is not mistaken for the
// body, and handles expression-bodied arrows. Returns None when there is no callback.
fn callback_body(src: &str, call_idx: usize) -> Option<&str> {
    let open = src[call_idx..].find('(')? + call_idx;
    let bytes = src.as_bytes();
    let mut depth = 0i32;
    let mut end = open;
    for (e, &b) in bytes.iter().enumerate().skip(open) {
        if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                end = e;
                break;
            }
        }
    }
    let region = &src[open..=end];
    let rel = ARROW_OR_FN.find(region)?.start();
    let after_arrow = open
        + rel
        + if region[rel..].starts_with("=>") {
            2
        } else {
            0
        };
    if let Some(brace) = src[after_arrow..].find('{').map(|p| after_arrow + p)
        && brace < end
        && let Some(span) = block_span(src, after_arrow)
    {
        return Some(&src[span.0..span.1]);
    }
    Some(&src[after_arrow..end]) // expression-bodied arrow
}

#[cfg(test)]
pub fn inspect_ts(path: &str, raw: &str) -> Vec<String> {
    inspect_ts_with(path, raw, None)
}

fn inspect_ts_with(path: &str, raw: &str, helpers: Option<&Regex>) -> Vec<String> {
    let src = mask_comments_and_strings(raw);
    let mut failures = vec![];

    for m in TS_SKIP.find_iter(&src) {
        failures.push(format!(
            "{path}:{}: committed skipped/todo test ({}) — a skipped test is a false green",
            line_of(&src, m.start()),
            m.as_str()
        ));
    }
    for m in TS_FOCUS.find_iter(&src) {
        failures.push(format!(
            "{path}:{}: focused test ({}) — suppresses the rest of the suite",
            line_of(&src, m.start()),
            m.as_str()
        ));
    }
    for m in TS_EXIT.find_iter(&src) {
        failures.push(format!(
            "{path}:{}: process.exit(0) in a test file — exits green without an assertion result",
            line_of(&src, m.start())
        ));
    }
    for m in TS_TEST_CALL.find_iter(&src) {
        let Some(body) = callback_body(&src, m.start()) else {
            continue;
        };
        if !TS_ASSERTION.is_match(body) && !asserts_via_helper(body, helpers) {
            failures.push(format!(
                "{path}:{}: test has no assertion (no expect/assert) — it passes on any behavior",
                line_of(&src, m.start())
            ));
        }
    }
    failures.extend(ts_tautologies(path, &src));
    failures.extend(ts_swallowed_assertions(path, &src));
    failures.extend(ts_fire_and_forget(path, &src));
    failures
}

// Start of the statement containing `idx`: scan backwards to the previous `;`, `{`, or
// `}` OUTSIDE any balanced group — an object literal or callback inside the statement
// (`await make({}).then(…)`, `.then(r => { … }).catch(…)`) must not cut it short (Codex
// round 5). Newlines are not statement boundaries — promise chains span lines.
fn statement_start(src: &str, idx: usize) -> usize {
    let b = src.as_bytes();
    let (mut paren, mut brace, mut bracket) = (0i32, 0i32, 0i32);
    for i in (0..idx).rev() {
        match b[i] {
            b')' => paren += 1,
            b']' => bracket += 1,
            b'}' => {
                if paren == 0 && bracket == 0 && brace == 0 {
                    return i + 1; // end of the previous block
                }
                brace += 1;
            }
            b'(' if paren > 0 => paren -= 1,
            b'[' if bracket > 0 => bracket -= 1,
            b'{' if brace > 0 => brace -= 1,
            b'{' if paren == 0 && bracket == 0 => return i + 1, // enclosing block opens
            b';' if paren == 0 && brace == 0 && bracket == 0 => return i + 1,
            _ => {}
        }
    }
    0
}

// A failed assertion whose error is swallowed still passes the test: `try { expect(..) }
// catch {}` and `chain-with-expect.catch(() => {})`. Only assertion-carrying try bodies
// and chains are flagged — an empty catch around cleanup (`try { rm(tmp) } catch {}`) is
// not an assertion swallow.
fn ts_swallowed_assertions(path: &str, src: &str) -> Vec<String> {
    let mut out = vec![];
    for m in TS_TRY.find_iter(src) {
        let Some(span) = block_span(src, m.start()) else {
            continue;
        };
        if TS_ASSERTION.is_match(&src[span.0..span.1]) && EMPTY_CATCH_AT.is_match(&src[span.1..]) {
            out.push(format!(
                "{path}:{}: assertion inside try with an empty catch — a failed expect is swallowed and the test passes",
                line_of(src, m.start())
            ));
        }
    }
    for m in TS_DOT_CATCH_EMPTY.find_iter(src) {
        let stmt = &src[statement_start(src, m.start())..m.start()];
        if TS_ASSERTION.is_match(stmt) {
            out.push(format!(
                "{path}:{}: .catch(() => {{}}) on an assertion chain — a rejected expect resolves clean and the test passes",
                line_of(src, m.start())
            ));
        }
    }
    out
}

// Drop every BALANCED `( … )` group, so operators inside completed conditions and loop
// headers do not read as part of the statement head. An unclosed group's content is
// kept: the head ends mid-expression at the `.then`, so an unclosed paren is the chain's
// own enclosing expression — `while ((p = f().then(…` must keep its `=` so a genuinely
// assigned chain still suppresses (Codex round 7).
fn strip_parens(s: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(s.len());
    let mut starts: Vec<usize> = vec![];
    for c in s.chars() {
        match c {
            '(' => {
                starts.push(out.len());
                out.push(c);
            }
            ')' => {
                if let Some(start) = starts.pop() {
                    out.truncate(start);
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out.into_iter().collect()
}

// End of the statement containing `idx`: the next `;` outside any balanced group, or the
// `}` that closes the enclosing block.
fn statement_end(src: &str, idx: usize) -> usize {
    let b = src.as_bytes();
    let (mut paren, mut brace, mut bracket) = (0i32, 0i32, 0i32);
    for (i, &c) in b.iter().enumerate().skip(idx) {
        match c {
            b'(' => paren += 1,
            b'[' => bracket += 1,
            b'{' => brace += 1,
            b')' if paren > 0 => paren -= 1,
            b']' if bracket > 0 => bracket -= 1,
            b'}' => {
                if brace == 0 {
                    return i; // enclosing block closes
                }
                brace -= 1;
            }
            b';' if paren == 0 && brace == 0 && bracket == 0 => return i,
            _ => {}
        }
    }
    src.len()
}

// The fire-and-forget false green: `p.then(r => expect(r).toBe(1));` inside a test. The
// runner finishes before the callback runs, so the assertion can never fail the test.
// Awaited, returned, or assigned chains are fine. One verdict per statement: a chain's
// first `.then` anchors it, and the assertion may sit in any later link.
fn ts_fire_and_forget(path: &str, src: &str) -> Vec<String> {
    let mut out = vec![];
    let mut seen_statements = std::collections::HashSet::new();
    for m in TS_DOT_THEN.find_iter(src) {
        let stmt = statement_start(src, m.start());
        if !seen_statements.insert(stmt) {
            continue; // a later .then in a chain already judged with the first
        }
        let head = src[stmt..m.start()].trim_start();
        // Strip parenthesized groups (a `for (let i = 0; …)` header or an `if (…)`
        // condition is not an assignment of the chain — Codex round 6), then arrows and
        // comparison operators, before looking for a real assignment.
        let assigned = strip_parens(head)
            .replace("=>", "")
            .replace("===", "")
            .replace("!==", "")
            .replace("==", "")
            .replace("!=", "")
            .replace(">=", "")
            .replace("<=", "")
            .contains('=');
        if head.starts_with("await") || head.starts_with("return") || assigned {
            continue;
        }
        // The assertion may be in any link of the chain, so scan to the statement's end.
        let end = statement_end(src, m.start());
        if TS_ASSERTION.is_match(&src[m.start()..end]) {
            out.push(format!(
                "{path}:{}: unawaited promise chain contains an assertion — the test finishes before it runs, so it can never fail; await or return the chain",
                line_of(src, m.start())
            ));
        }
    }
    out
}

fn ts_tautologies(path: &str, src: &str) -> Vec<String> {
    let mut out = vec![];
    for c in TS_EXPECT_SELF.captures_iter(src) {
        // `expect(true).toBe(true)` belongs to the dedicated rule below — reporting it
        // here too double-counts the same line.
        if c.get(1).map(|m| m.as_str()) == c.get(2).map(|m| m.as_str()) && &c[1] != "true" {
            let m = c.get(0).unwrap();
            out.push(format!(
                "{path}:{}: expect({}) compared to itself — tautological",
                line_of(src, m.start()),
                &c[1]
            ));
        }
    }
    for m in TS_EXPECT_TRUE.find_iter(src) {
        out.push(format!(
            "{path}:{}: expect(true) is tautological",
            line_of(src, m.start())
        ));
    }
    out
}

// Any .rs file may hold inline #[test] functions, so all are scanned (a no-op without
// #[test]). For TS/JS, restrict to test/spec files so production it()/test() calls are
// not mistaken for tests.
fn is_test_file(p: &str) -> bool {
    p.ends_with(".rs") || TS_TEST_FILE.is_match(p)
}

// A scan is only trustworthy when it says what it actually read: `errors` are files or
// directories the scan could NOT verify (unreadable, invalid UTF-8, walk failure) — a
// clean verdict with a non-empty error list is not clean (Codex round 3: unreadable
// input silently read as "no smells").
pub struct Scan {
    pub failures: Vec<String>,
    pub files_scanned: usize,
    pub errors: Vec<String>,
}

fn inspect_file(path: &Path, helpers: Option<&Regex>, scan: &mut Scan) {
    let p = path.to_string_lossy();
    let raw = match std::fs::read_to_string(path) {
        Ok(r) => r,
        Err(e) => {
            scan.errors.push(format!("cannot read {p}: {e}"));
            return;
        }
    };
    let failures = match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => inspect_rust_with(&p, &raw, helpers),
        Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") => inspect_ts_with(&p, &raw, helpers),
        _ => return, // not a scannable test file; does not count as scanned
    };
    scan.files_scanned += 1;
    scan.failures.extend(failures);
}

fn walk(dir: &Path, errors: &mut Vec<String>) -> Vec<std::path::PathBuf> {
    let mut files = vec![];
    // Follow symlinks: a symlinked test tree silently skipped is unverified code reading
    // as clean (Codex round 5). A symlink loop surfaces as a walk error, which fails the
    // scan closed rather than looping.
    for entry in WalkDir::new(dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            !matches!(
                e.file_name().to_str(),
                Some("node_modules") | Some("target") | Some(".git")
            )
        })
    {
        match entry {
            Ok(e) => {
                if e.file_type().is_file() && is_test_file(&e.path().to_string_lossy()) {
                    files.push(e.into_path());
                }
            }
            Err(e) => errors.push(format!("cannot walk {}: {e}", dir.display())),
        }
    }
    files
}

pub fn inspect_paths(paths: &[String], helper_names: &[String]) -> Scan {
    let helpers = helper_matcher(helper_names);
    let mut scan = Scan {
        failures: vec![],
        files_scanned: 0,
        errors: vec![],
    };
    for p in paths {
        let path = Path::new(p);
        let files = if path.is_dir() {
            walk(path, &mut scan.errors)
        } else {
            vec![path.to_path_buf()]
        };
        for f in files {
            inspect_file(&f, helpers.as_ref(), &mut scan);
        }
    }
    scan
}

#[cfg(test)]
#[path = "smells_tests.rs"]
mod tests;
