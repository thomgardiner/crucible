use super::*;

fn run(exit: i32, output: &str) -> RunResult {
    RunResult {
        exit,
        output: output.to_string(),
        timed_out: false,
    }
}

fn timed_out(exit: i32) -> RunResult {
    RunResult {
        exit,
        output: "hung".into(),
        timed_out: true,
    }
}

fn pat() -> Regex {
    Regex::new(r"FAILED (\w+)").unwrap()
}

#[test]
fn repeated_timeouts_are_inconclusive_not_stable() {
    // A run that never finished proves nothing about determinism (Codex P1 #8).
    let runs = vec![timed_out(124), timed_out(124)];
    let r = analyze(&runs, Some(&pat()));
    assert_eq!(r.verdict, "inconclusive");
    assert_eq!(r.timed_out_runs, 2);
}

#[test]
fn identical_passing_runs_are_stable() {
    let runs = vec![run(0, "ok"), run(0, "ok"), run(0, "ok")];
    let r = analyze(&runs, Some(&pat()));
    assert_eq!(r.verdict, "stable");
    assert!(r.flaky_tests.is_empty());
    assert!(!r.exit_inconsistent);
}

#[test]
fn a_test_that_fails_in_some_runs_is_flaky() {
    let runs = vec![run(0, "ok"), run(1, "FAILED test_x"), run(0, "ok")];
    let r = analyze(&runs, Some(&pat()));
    assert_eq!(r.verdict, "flaky");
    assert_eq!(r.flaky_tests, vec!["test_x".to_string()]);
    assert!(r.exit_inconsistent, "the exit code differed too");
}

#[test]
fn a_consistently_failing_test_is_not_flaky_but_is_recorded_as_red() {
    // Failed in every run: deterministic, so not flaky, but it IS red — the CLI reads
    // failing_every_run to refuse a success even when exit codes agree (Codex round 3).
    let runs = vec![run(1, "FAILED test_x"), run(1, "FAILED test_x")];
    let r = analyze(&runs, Some(&pat()));
    assert!(r.flaky_tests.is_empty());
    assert_eq!(r.failing_every_run, vec!["test_x".to_string()]);
    assert_eq!(r.verdict, "stable");
}

#[test]
fn a_test_reported_failing_every_run_while_exit_0_is_still_red() {
    // The exit-code check alone misses this: a suite that prints a failure every run but
    // exits 0. failing_every_run catches it so the CLI does not certify a passing run.
    let runs = vec![run(0, "FAILED critical"), run(0, "FAILED critical")];
    let r = analyze(&runs, Some(&pat()));
    assert!(!r.exit_inconsistent);
    assert_eq!(r.failing_every_run, vec!["critical".to_string()]);
}

#[test]
fn inconsistent_exit_codes_alone_flag_flaky() {
    // No fail pattern: fall back to exit-code agreement.
    let runs = vec![run(0, "..."), run(1, "...")];
    let r = analyze(&runs, None);
    assert!(r.exit_inconsistent);
    assert_eq!(r.verdict, "flaky");
}

#[test]
fn a_single_run_cannot_detect_flake() {
    let r = analyze(&[run(1, "FAILED test_x")], Some(&pat()));
    assert_eq!(r.verdict, "stable");
}
