use super::*;
use crate::config::{MutationRecipe, Waiver};
use crate::proc::Output;
use serde_json::json;
use std::cell::Cell;
use std::path::Path;

const CARGO_OUTPUT: &str = "
Found 12 mutants to test
CAUGHT   crates/engine/src/retry.rs:20:1: replace is_blocking -> true
MISSED   crates/engine/src/decision.rs:88:9: replace > with >= in should_buy
TIMEOUT  crates/engine/src/loop.rs:5:1: replace body with ()
MISSED   crates/modules/target/src/checkout.rs:142:5: delete ! in validate
12 mutants tested: 9 caught, 2 missed, 1 timeout
";

struct FakeExec {
    code: i32,
    output: String,
    timed_out: bool,
    memory_exceeded: bool,
    output_exceeded: bool,
    ran: Cell<bool>,
    memory_bytes: Cell<Option<u64>>,
}

impl FakeExec {
    fn new(code: i32, output: &str, timed_out: bool) -> Self {
        Self {
            code,
            output: output.to_string(),
            timed_out,
            memory_exceeded: false,
            output_exceeded: false,
            ran: Cell::new(false),
            memory_bytes: Cell::new(None),
        }
    }
}

impl Exec for FakeExec {
    fn run(&self, _cmd: &str, _cwd: &Path, _timeout: Duration) -> Output {
        self.ran.set(true);
        Output {
            code: self.code,
            output: self.output.clone(),
            timed_out: self.timed_out,
            memory_exceeded: self.memory_exceeded,
            output_exceeded: self.output_exceeded,
        }
    }

    fn run_limited(&self, cmd: &str, cwd: &Path, timeout: Duration, memory_bytes: u64) -> Output {
        self.memory_bytes.set(Some(memory_bytes));
        self.run(cmd, cwd, timeout)
    }
}

fn recipe() -> MutationRecipe {
    serde_json::from_value(json!({ "cmd": "mutants" })).unwrap()
}

fn waivers(v: serde_json::Value) -> Vec<Waiver> {
    serde_json::from_value(v).unwrap()
}

fn run(recipe: &MutationRecipe, w: &[Waiver], high_risk: bool, exec: &FakeExec) -> HardenResult {
    run_harden(recipe, Path::new("."), w, high_risk, exec)
}

#[test]
fn parse_survivors_counts_missed_and_timeout() {
    let s = parse_survivors(CARGO_OUTPUT, CARGO_MUTANTS_SURVIVED);
    assert_eq!(s.len(), 3);
    assert_eq!(
        s[0],
        Survivor {
            file: "crates/engine/src/decision.rs".into(),
            line: 88,
            mutation: "replace > with >= in should_buy".into(),
        }
    );
    assert!(
        s.iter().any(|x| x.file == "crates/engine/src/loop.rs"),
        "the TIMEOUT mutant counts"
    );
    assert!(
        s.iter()
            .any(|x| x.file == "crates/modules/target/src/checkout.rs")
    );
}

#[test]
fn parse_survivors_does_not_hang_on_zero_width_pattern() {
    assert_eq!(parse_survivors("a\nb\nc", "(?:)"), vec![]);
}

#[test]
fn parse_survivors_handles_paths_with_spaces_and_drive_letters() {
    // Codex P1 #9: the default pattern rejected spaces and Windows drive colons.
    let out =
        "MISSED   my crate/src/pay.rs:10: replace + with -\nTIMEOUT  C:\\a\\b.rs:3: delete body\n";
    let s = parse_survivors(out, CARGO_MUTANTS_SURVIVED);
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].file, "my crate/src/pay.rs");
    assert_eq!(s[0].line, 10);
    assert_eq!(s[1].file, "C:\\a\\b.rs");
    assert_eq!(s[1].line, 3);
}

#[test]
fn validate_waivers_requires_a_reason_and_a_matcher() {
    assert_eq!(
        validate_waivers(&waivers(
            json!([{ "file": "a.rs", "line": 1, "reason": "equivalent: no-op reorder" }])
        )),
        Vec::<String>::new()
    );
    let bad = validate_waivers(&waivers(json!([{ "file": "a.rs", "line": 1 }])));
    assert!(bad.iter().any(|f| f.contains("missing \"reason\"")));
    let no_match = validate_waivers(&waivers(json!([{ "reason": "x" }])));
    assert!(no_match.iter().any(|f| f.contains("missing \"file\"")));
}

#[test]
fn empty_reason_or_empty_substring_waivers_are_rejected() {
    // Codex P0 #5: reason:"" and mutation:"" (which matches everything) must not pass.
    let empty_reason = validate_waivers(&waivers(
        json!([{ "file": "a.rs", "line": 1, "reason": "" }]),
    ));
    assert!(
        empty_reason.iter().any(|f| f.contains("reason")),
        "{empty_reason:?}"
    );
    let empty_sub = validate_waivers(&waivers(
        json!([{ "file": "a.rs", "mutation": "", "reason": "x" }]),
    ));
    assert!(
        empty_sub.iter().any(|f| f.contains("non-empty")),
        "{empty_sub:?}"
    );
}

#[test]
fn apply_waivers_dismisses_a_match_and_keeps_the_rest() {
    let survivors = parse_survivors(CARGO_OUTPUT, CARGO_MUTANTS_SURVIVED);
    let w = waivers(
        json!([{ "file": "crates/engine/src/decision.rs", "line": 88, "reason": "equivalent: >= identical here" }]),
    );
    let (live, dismissed) = apply_waivers(survivors, &w);
    assert_eq!(dismissed.len(), 1);
    assert_eq!(live.len(), 2);
    assert!(
        live.iter()
            .any(|s| s.file == "crates/modules/target/src/checkout.rs")
    );
}

#[test]
fn a_waiver_can_match_by_mutation_substring() {
    let survivors = parse_survivors(CARGO_OUTPUT, CARGO_MUTANTS_SURVIVED);
    let w = waivers(
        json!([{ "file": "crates/engine/src/decision.rs", "mutation": "replace > with >=", "reason": "equivalent" }]),
    );
    let (live, _) = apply_waivers(survivors, &w);
    assert_eq!(live.len(), 2);
}

#[test]
fn classify_is_tiered() {
    assert_eq!(classify(1, true), "block");
    assert_eq!(classify(1, false), "advisory");
    assert_eq!(classify(0, true), "pass");
}

#[test]
fn survivor_report_names_the_next_test() {
    let r = survivor_report(&[Survivor {
        file: "a.rs".into(),
        line: 5,
        mutation: "replace + with -".into(),
    }]);
    assert!(
        r[0].instruction
            .contains("Write a test that fails under this mutation")
    );
    assert_eq!(r[0].line, 5);
}

#[test]
fn harden_blocks_on_live_survivor_in_high_risk_code() {
    let r = run(&recipe(), &[], true, &FakeExec::new(3, CARGO_OUTPUT, false));
    assert_eq!(r.verdict, "block");
    assert_eq!(r.report.len(), 3);
}

#[test]
fn harden_blocks_on_a_timed_out_mutant_even_when_missed_are_waived() {
    let output = "CAUGHT   a.rs:1:1: x\nMISSED   a.rs:2:1: y\nTIMEOUT  a.rs:3:1: z\n";
    let w = waivers(json!([{ "file": "a.rs", "line": 2, "reason": "equivalent" }]));
    let r = run(&recipe(), &w, true, &FakeExec::new(3, output, false));
    assert_eq!(r.verdict, "block");
    assert_eq!(r.report.len(), 1);
    assert_eq!(r.report[0].line, 3);
}

#[test]
fn harden_is_advisory_outside_high_risk_code() {
    let r = run(
        &recipe(),
        &[],
        false,
        &FakeExec::new(3, CARGO_OUTPUT, false),
    );
    assert_eq!(r.verdict, "advisory");
}

#[test]
fn harden_passes_when_every_mutant_is_caught_or_waived() {
    let w = waivers(json!([
        { "file": "crates/engine/src/decision.rs", "line": 88, "reason": "equivalent" },
        { "file": "crates/engine/src/loop.rs", "line": 5, "reason": "equivalent: known slow path" },
        { "file": "crates/modules/target/src/checkout.rs", "line": 142, "reason": "equivalent" },
    ]));
    let r = run(&recipe(), &w, true, &FakeExec::new(3, CARGO_OUTPUT, false));
    assert_eq!(r.verdict, "pass");
}

#[test]
fn harden_fails_closed_on_a_timed_out_run() {
    let r = run(&recipe(), &[], false, &FakeExec::new(124, "partial", true));
    assert_eq!(r.verdict, "block");
    assert!(r.error.unwrap().contains("timed out"));
}

#[test]
fn harden_caps_the_mutation_process_tree_and_fails_closed_on_exhaustion() {
    let mut exec = FakeExec::new(125, "", false);
    exec.memory_exceeded = true;
    let result = run(&recipe(), &[], true, &exec);

    assert_eq!(exec.memory_bytes.get(), Some(2048 * 1024 * 1024));
    assert_eq!(result.verdict, "block");
    assert_eq!(
        result.error.as_deref(),
        Some("mutation run exceeded its 2048 MiB memory limit")
    );
}

#[test]
fn harden_fails_closed_when_the_run_is_killed_for_runaway_output() {
    // A mutation run killed to protect the disk did not finish, so it certifies nothing.
    let mut exec = FakeExec::new(126, "flooded", false);
    exec.output_exceeded = true;
    let result = run(&recipe(), &[], true, &exec);

    assert_eq!(result.verdict, "block");
    assert!(
        result.error.unwrap().contains("runaway output"),
        "must name the disk guard"
    );
}

#[test]
fn harden_rejects_a_disabled_memory_budget() {
    let recipe: MutationRecipe = serde_json::from_value(json!({
        "cmd": "mutants",
        "memoryMb": 0
    }))
    .unwrap();
    let exec = FakeExec::new(0, CARGO_OUTPUT, false);

    let result = run(&recipe, &[], true, &exec);

    assert!(!exec.ran.get());
    assert_eq!(result.verdict, "block");
    assert_eq!(
        result.error.as_deref(),
        Some("memoryMb must be a positive MiB value")
    );
}

#[test]
fn harden_fails_closed_when_the_command_never_ran_a_mutation_pass() {
    // Codex round 2 #2: `true` exits 0 with no output — no survivors, but also no evidence
    // any mutation ran. That must block, not pass as "every mutant caught".
    let r = run(&recipe(), &[], true, &FakeExec::new(0, "", false));
    assert_eq!(r.verdict, "block");
    assert!(
        r.error
            .unwrap()
            .contains("no evidence the mutation run completed"),
        "a no-op command must not certify a clean mutation result"
    );
}

#[test]
fn harden_passes_a_clean_run_that_printed_its_summary() {
    // A real cargo-mutants pass with everything caught exits 0 and prints its summary; the
    // completion evidence lets a genuine zero-survivor result pass.
    let clean = "Found 3 mutants to test\nCAUGHT a.rs:1:1: x\n3 mutants tested: 3 caught\n";
    let r = run(&recipe(), &[], true, &FakeExec::new(0, clean, false));
    assert_eq!(r.verdict, "pass");
}

#[test]
fn harden_fails_closed_when_tool_errors_but_nothing_parses() {
    let r = run(
        &recipe(),
        &[],
        false,
        &FakeExec::new(2, "some unrecognized output", false),
    );
    assert_eq!(r.verdict, "block");
    assert!(r.error.unwrap().contains("no survivors parsed"));
}

#[test]
fn harden_rejects_invalid_waivers_before_running() {
    let exec = FakeExec::new(0, "", false);
    let w = waivers(json!([{ "file": "a", "line": 1 }]));
    let r = run(&recipe(), &w, false, &exec);
    assert_eq!(r.verdict, "block");
    assert!(!exec.ran.get(), "must reject waivers before spawning");
    assert!(r.waiver_failures.iter().any(|f| f.contains("reason")));
}

#[test]
fn harden_rejects_invalid_survivor_pattern_instead_of_certifying_clean() {
    // A broken pattern used to parse as zero survivors; with exit 0 that looked green.
    let mut rec = recipe();
    rec.survivor_pattern = Some("([unclosed".into());
    let clean = "Found 1 mutants to test\nMISSED a.rs:1:1: x\n1 mutants tested: 1 missed\n";
    let r = run(&rec, &[], true, &FakeExec::new(0, clean, false));
    assert_eq!(r.verdict, "block");
    assert!(
        r.error.unwrap().contains("survivorPattern"),
        "must name the pattern failure"
    );
}

#[test]
fn harden_refuses_to_certify_when_zero_mutants_were_tested() {
    let out = "Found 0 mutants to test\n0 mutants tested: 0 caught\n";
    let r = run(&recipe(), &[], true, &FakeExec::new(0, out, false));
    assert_eq!(r.verdict, "block");
    assert!(
        r.error.unwrap().contains("0 mutants"),
        "must refuse empty mutation scope"
    );
}

#[test]
fn harden_refuses_to_certify_when_every_mutant_is_unviable() {
    // Unviable-only is not "tests bit" — nothing executed under mutation.
    let out = "Found 1 mutant to test\nok       Unmutated baseline\n1 mutant tested in 1s: 1 unviable\n WARN No mutants were viable\n";
    let r = run(&recipe(), &[], true, &FakeExec::new(0, out, false));
    assert_eq!(r.verdict, "block");
    assert!(
        r.error.unwrap().contains("no viable mutants"),
        "must refuse all-unviable"
    );
}

#[test]
fn harden_refuses_unviable_summary_without_the_warn_banner() {
    // Second branch of the unviable gate: summary says unviable, no MISSED/caught,
    // even when cargo-mutants omits the "No mutants were viable" banner.
    let out = "Found 2 mutants to test\nok       Unmutated baseline\n2 mutants tested in 3s: 2 unviable\n";
    let r = run(&recipe(), &[], true, &FakeExec::new(0, out, false));
    assert_eq!(r.verdict, "block");
    assert!(
        r.error.as_ref().unwrap().contains("no viable mutants"),
        "{:?}",
        r.error
    );
}

#[test]
fn harden_does_not_treat_a_clean_caught_run_as_unviable() {
    // Positive control for the unviable gate: a real "all caught" summary must pass.
    let out = "Found 2 mutants to test\nok       Unmutated baseline\n2 mutants tested: 0 missed, 2 caught\n";
    let r = run(&recipe(), &[], true, &FakeExec::new(0, out, false));
    assert_eq!(r.verdict, "pass", "clean caught run must not hit unviable gate: {:?}", r.error);
    assert!(r.error.is_none());
    assert!(r.report.is_empty());
}

#[test]
fn harden_does_not_treat_mixed_caught_and_unviable_as_all_unviable() {
    // Some unviable + some caught, no MISSED lines: still a real test run. The
    // `caught || MISSED` early-exit must stay OR — an && mutant would block this.
    let out = "Found 3 mutants to test\nok       Unmutated baseline\n3 mutants tested: 0 missed, 1 caught, 2 unviable\n";
    let r = run(&recipe(), &[], true, &FakeExec::new(0, out, false));
    assert_eq!(
        r.verdict, "pass",
        "mixed caught+unviable must pass: {:?}",
        r.error
    );
}
