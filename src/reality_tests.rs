use super::*;
use crate::config::Recipe;
use crate::proc::Output;
use serde_json::json;
use std::fs;
use std::path::Path;

// A deterministic fake command runner keyed on a substring of the command.
struct FakeExec {
    table: Vec<(&'static str, i32, &'static str, bool)>,
}

impl Exec for FakeExec {
    fn run(&self, cmd: &str, _cwd: &Path, _timeout: Duration) -> Output {
        for (needle, code, output, timed_out) in &self.table {
            if cmd.contains(needle) {
                return Output {
                    code: *code,
                    output: (*output).to_string(),
                    timed_out: *timed_out,
                    memory_exceeded: false,
                    output_exceeded: false,
                };
            }
        }
        Output {
            code: 0,
            output: String::new(),
            timed_out: false,
            memory_exceeded: false,
            output_exceeded: false,
        }
    }
}

fn fake(table: Vec<(&'static str, i32, &'static str, bool)>) -> FakeExec {
    FakeExec { table }
}

fn recipe() -> Recipe {
    serde_json::from_value(json!({
        "repo": "demo",
        "build": { "cmd": "build-it", "timeoutSec": 1 },
        "boot": { "cmd": "boot-it", "timeoutSec": 1, "oracle": { "stdoutMatch": "window loaded" } },
        "drive": [{ "name": "checkout-replay", "cmd": "drive-it", "oracle": { "stdoutMatch": "ORDER PLACED" } }],
    }))
    .unwrap()
}

#[test]
fn no_boot_readiness_oracle_is_broken() {
    let mut r = recipe();
    r.boot = serde_json::from_value(json!({ "cmd": "boot-it" })).unwrap();
    let rep = run_crucible(Path::new("/x"), &r, &fake(vec![]), None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(
        rep.summary.contains("no readiness oracle"),
        "{}",
        rep.summary
    );
}

#[test]
fn no_drive_steps_is_broken() {
    let mut r = recipe();
    r.drive = vec![];
    let rep = run_crucible(Path::new("/x"), &r, &fake(vec![]), None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(rep.summary.contains("no critical paths"));
}

#[test]
fn empty_boot_oracle_is_broken() {
    // An empty stdoutMatch matches any output, so it is no oracle at all (Codex P0 #1).
    let mut r = recipe();
    r.boot = serde_json::from_value(json!({ "cmd": "boot-it", "oracle": { "stdoutMatch": "" } }))
        .unwrap();
    let rep = run_crucible(
        Path::new("/x"),
        &r,
        &fake(vec![("boot-it", 0, "anything at all", false)]),
        None,
    );
    assert_eq!(rep.verdict, "BROKEN");
    assert!(
        rep.summary.contains("no readiness oracle"),
        "{}",
        rep.summary
    );
}

#[test]
fn drive_step_without_oracle_is_broken() {
    // A drive step with no oracle passes on any exit-0 command that does nothing (Codex P0 #1).
    let mut r = recipe();
    r.drive = serde_json::from_value(json!([{ "name": "d", "cmd": "drive-it" }])).unwrap();
    let exec = fake(vec![
        ("boot-it", 0, "window loaded", false),
        ("drive-it", 0, "x", false),
    ]);
    let rep = run_crucible(Path::new("/x"), &r, &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(rep.summary.contains("no oracle"), "{}", rep.summary);
}

#[test]
fn a_boot_oracle_that_matches_empty_output_is_broken() {
    // ".*" is non-empty but matches empty output, so `true` (exit 0, no output) would
    // "pass" readiness. A real oracle must consume real output (Codex round 2 #1).
    let mut r = recipe();
    r.boot = serde_json::from_value(json!({ "cmd": "boot-it", "oracle": { "stdoutMatch": ".*" } }))
        .unwrap();
    let rep = run_crucible(
        Path::new("/x"),
        &r,
        &fake(vec![("boot-it", 0, "", false)]),
        None,
    );
    assert_eq!(rep.verdict, "BROKEN");
    assert!(
        rep.summary.contains("matches empty output"),
        "{}",
        rep.summary
    );
}

#[test]
fn an_invalid_forbid_regex_is_broken_not_silently_ignored() {
    // A stdoutForbid that does not compile must fail closed; the old code swallowed the
    // compile error and treated it as "did not match", disabling the check (Codex #1).
    let mut r = recipe();
    r.drive = serde_json::from_value(
        json!([{ "name": "d", "cmd": "drive-it", "oracle": { "stdoutMatch": "ORDER PLACED", "stdoutForbid": "[" } }]),
    )
    .unwrap();
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 0, "window loaded", false),
        ("drive-it", 0, "ORDER PLACED", false),
    ]);
    let rep = run_crucible(Path::new("/x"), &r, &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(rep.summary.contains("stdoutForbid"), "{}", rep.summary);
}

#[test]
fn a_forbid_only_drive_step_is_broken() {
    // "did not print ERROR" on empty output is not proof a flow ran — a drive step needs a
    // positive stdoutMatch, not a bare forbid (Codex round 2 #1).
    let mut r = recipe();
    r.drive = serde_json::from_value(
        json!([{ "name": "d", "cmd": "drive-it", "oracle": { "stdoutForbid": "ERROR" } }]),
    )
    .unwrap();
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 0, "window loaded", false),
        ("drive-it", 0, "", false),
    ]);
    let rep = run_crucible(Path::new("/x"), &r, &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(rep.summary.contains("no oracle"), "{}", rep.summary);
}

#[test]
fn builds_boots_and_drives_is_runs() {
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 0, "starting…\nwindow loaded\n", false),
        ("drive-it", 0, "ORDER PLACED #123", false),
    ]);
    let rep = run_crucible(Path::new("/x"), &recipe(), &exec, None);
    assert_eq!(rep.verdict, "RUNS");
    assert!(rep.build.unwrap().ok && rep.boot.unwrap().ok && rep.drive[0].ok);
}

#[test]
fn a_build_failure_stops_before_boot() {
    let exec = fake(vec![("build-it", 1, "error[E0432]", false)]);
    let rep = run_crucible(Path::new("/x"), &recipe(), &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    assert_eq!(rep.summary, "does not build");
    assert!(rep.boot.is_none());
}

#[test]
fn boots_to_exit_0_but_never_ready_is_broken() {
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 0, "starting…\npanic: thread main\n", false),
    ]);
    let rep = run_crucible(Path::new("/x"), &recipe(), &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    // Exit 0 without the readiness oracle is reported as never-ready, not as a crash —
    // the verdict must state what actually happened (Codex round 5).
    assert!(
        rep.summary.contains("never printed its readiness oracle"),
        "{}",
        rep.summary
    );
    assert_eq!(rep.drive.len(), 0);
}

#[test]
fn a_nonzero_boot_exit_is_a_crash() {
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 101, "thread 'main' panicked\n", false),
    ]);
    let rep = run_crucible(Path::new("/x"), &recipe(), &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(rep.summary.contains("crashed on launch"), "{}", rep.summary);
}

#[test]
fn a_boot_timeout_is_not_ready() {
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 124, "hung", true),
    ]);
    let rep = run_crucible(Path::new("/x"), &recipe(), &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(
        rep.summary.contains("did not reach ready"),
        "{}",
        rep.summary
    );
}

#[test]
fn boots_but_a_real_flow_fails_names_the_flow() {
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 0, "window loaded", false),
        ("drive-it", 1, "DECLINED", false),
    ]);
    let rep = run_crucible(Path::new("/x"), &recipe(), &exec, None);
    assert_eq!(rep.verdict, "BROKEN");
    assert!(rep.summary.contains("checkout-replay"), "{}", rep.summary);
}

#[test]
fn trust_audit_counts_mock_boundary_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("frontend/tests")).unwrap();
    fs::create_dir_all(root.join("crates/engine")).unwrap();
    fs::write(
        root.join("frontend/tests/a.test.ts"),
        "mockIPC(() => {});\nexpect(x).toBe(1);\n",
    )
    .unwrap();
    fs::write(
        root.join("frontend/tests/b.test.ts"),
        "renderReal(); expect(y).toBe(2);\n",
    )
    .unwrap();
    fs::write(
        root.join("crates/engine/flow_tests.rs"),
        "let db = Database::open_in_memory();\n",
    )
    .unwrap();
    let cfg: TrustCfg = serde_json::from_value(json!({
        "testRoots": ["frontend/tests", "crates"],
        "testPattern": "(\\.test\\.[tj]sx?$|_tests?\\.rs$)",
        "mockMarkers": ["mockIPC(", "Database::open_in_memory"],
    }))
    .unwrap();
    let trust = trust_audit(root, &cfg);
    assert_eq!(trust.total, 3);
    assert_eq!(trust.mocked, 2);
    assert_eq!(trust.real_boundary, 1);
    assert_eq!(trust.by_marker["mockIPC("], 1);
}

#[test]
fn format_report_renders_a_deterministic_crash() {
    let exec = fake(vec![("build-it", 101, "thread 'main' panicked", false)]);
    let rep = run_crucible(Path::new("/x"), &recipe(), &exec, None);
    let text = format_report(&rep);
    assert!(text.contains("VERDICT: BROKEN"));
    assert!(text.contains("FAIL  build the real artifact"));
    assert!(text.contains("panicked"));
}

#[test]
fn tail_str_truncates_only_past_the_limit_and_keeps_the_tail() {
    assert_eq!(tail_str("hello", 5), "hello", "exact length is untouched");
    assert_eq!(tail_str("hello", 4), "…ello", "keeps the LAST n chars");
    // n=2 distinguishes len-n from len/n (6-2=4 vs 6/2=3): another coincidence guard.
    assert_eq!(tail_str("abcdef", 2), "…ef");
    assert_eq!(tail_str("hi", 10), "hi");
}

#[test]
fn trust_audit_math_distinguishes_subtraction_from_division() {
    // 4 files, 1 mocked: real_boundary must be 3 (4-1), not 4 (4/1).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("t")).unwrap();
    for (name, body) in [
        ("a.test.ts", "mockIPC(); expect(1).toBe(1);\n"),
        ("b.test.ts", "expect(2).toBe(2);\n"),
        ("c.test.ts", "expect(3).toBe(3);\n"),
        ("d.test.ts", "expect(4).toBe(4);\n"),
    ] {
        fs::write(root.join("t").join(name), body).unwrap();
    }
    let cfg: TrustCfg = serde_json::from_value(json!({
        "testRoots": ["t"],
        "testPattern": "\\.test\\.ts$",
        "mockMarkers": ["mockIPC("],
    }))
    .unwrap();
    let trust = trust_audit(root, &cfg);
    assert_eq!(trust.total, 4);
    assert_eq!(trust.mocked, 1);
    assert_eq!(trust.real_boundary, 3);
}

#[test]
fn format_report_shows_the_trust_percentage_and_failing_drive_tail() {
    let mut r = recipe();
    r.trust = serde_json::from_value(json!({
        "testRoots": ["nowhere"],
        "testPattern": "x",
        "mockMarkers": ["m"],
    }))
    .unwrap();
    let exec = fake(vec![
        ("build-it", 0, "", false),
        ("boot-it", 0, "window loaded", false),
        ("drive-it", 1, "unique-crash-tail-text", false),
    ]);
    let mut rep = run_crucible(Path::new("/x"), &r, &exec, None);
    // Fixed trust numbers so the percentage math is pinned: 1 of 4 mocked = 25%.
    // 2/5 distinguishes (2*100)/5 = 40 from (2+100)/5 = 20 — pct math is pinned.
    rep.trust = Some(TrustReport {
        total: 5,
        mocked: 2,
        real_boundary: 3,
        by_marker: Default::default(),
    });
    let text = format_report(&rep);
    assert!(text.contains("2/5 test files (40%)"), "{text}");
    assert!(
        text.contains("--- checkout-replay output (tail) ---"),
        "{text}"
    );
    assert!(text.contains("unique-crash-tail-text"), "{text}");
}
