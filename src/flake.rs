//! The determinism check: nondeterminism detection. Run the suite N times and compare. A
//! test that fails in some runs and passes in others is flaky, and a flaky green is a false
//! green — the suite "passes" only by luck of ordering, timing, or shared state. This
//! detects disagreement between runs; agreement is necessary but not sufficient (a suite
//! that fails identically every run agrees, and is red, not passing — the CLI treats that
//! as a failure, not a clean verification).
//!
//! The analysis is pure (takes the captured runs), so it is tested without spawning; the
//! CLI does the actual N executions.

use regex::Regex;
use std::collections::BTreeSet;

pub struct RunResult {
    pub exit: i32,
    pub output: String,
    pub timed_out: bool,
}

pub struct FlakeReport {
    // Tests that failed in some runs but not all.
    pub flaky_tests: Vec<String>,
    // Tests that failed in EVERY run. Deterministic, but red — a suite reporting these is
    // failing, even if it exits 0, so it must never read as a passing verification.
    pub failing_every_run: Vec<String>,
    // The suite's exit code was not the same across every run.
    pub exit_inconsistent: bool,
    // Runs that did not finish. A timed-out run proves nothing about determinism.
    pub timed_out_runs: usize,
    // "stable" | "flaky" | "inconclusive"
    pub verdict: String,
}

fn failed_set(output: &str, pat: &Regex) -> BTreeSet<String> {
    pat.captures_iter(output)
        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
        .collect()
}

// A test is flaky when it is in the failed set of at least one run and absent from at
// least one other — failed sometimes, passed other times. A consistently failing test
// (failed in every run) is not flaky, it is just failing; a consistently passing one is
// stable. Requires at least two runs to say anything.
pub fn analyze(runs: &[RunResult], fail_pattern: Option<&Regex>) -> FlakeReport {
    let timed_out_runs = runs.iter().filter(|r| r.timed_out).count();
    if runs.len() < 2 {
        return FlakeReport {
            flaky_tests: vec![],
            failing_every_run: vec![],
            exit_inconsistent: false,
            timed_out_runs,
            verdict: "stable".into(),
        };
    }

    let first_exit = runs[0].exit;
    let exit_inconsistent = runs.iter().any(|r| r.exit != first_exit);

    let mut flaky_tests = vec![];
    let mut failing_every_run = vec![];
    if let Some(pat) = fail_pattern {
        let sets: Vec<BTreeSet<String>> = runs.iter().map(|r| failed_set(&r.output, pat)).collect();
        let union: BTreeSet<String> = sets.iter().flatten().cloned().collect();
        for name in union {
            let failed_in = sets.iter().filter(|s| s.contains(&name)).count();
            if failed_in == runs.len() {
                // Reported as failing in every run: deterministic, but red.
                failing_every_run.push(name);
            } else {
                // The union only holds names that failed in at least one run, so
                // anything not failing everywhere failed somewhere: flaky.
                flaky_tests.push(name);
            }
        }
    }

    // A run that timed out never finished, so the suite's determinism is unproven — that is
    // inconclusive, not stable. A real flake (a test that flips) still reports flaky.
    let verdict = if !flaky_tests.is_empty() || exit_inconsistent {
        "flaky"
    } else if timed_out_runs > 0 {
        "inconclusive"
    } else {
        "stable"
    };
    FlakeReport {
        flaky_tests,
        failing_every_run,
        exit_inconsistent,
        timed_out_runs,
        verdict: verdict.to_string(),
    }
}

#[cfg(test)]
#[path = "flake_tests.rs"]
mod tests;
