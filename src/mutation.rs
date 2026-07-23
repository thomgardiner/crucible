//! The mutation gate: the force function that makes an agent write correct tests. It
//! runs the repo's diff-scoped mutation tool, and a mutant that survives on changed
//! code is mechanical proof no test checks that behavior — coverage forces execution,
//! mutation forces assertion. Tiered like Google/Meta run it: mutate only changed
//! lines, let a human waive an equivalent mutant with a reason, block only where it
//! matters, and emit each survivor as the exact next test to write.

use crate::config::{MutationRecipe, Waiver};
use crate::proc::Exec;
use regex::RegexBuilder;
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

const DEFAULT_MEMORY_MB: u64 = 2048;
const MIB: u64 = 1024 * 1024;

// A survivor is any mutant the suite did not confirm killed — for cargo-mutants that
// is both MISSED (a test should have failed and didn't) and TIMEOUT (never confirmed
// caught). Groups: 1=file, 2=line, 3=mutation. A repo can override for Stryker etc.
pub const CARGO_MUTANTS_SURVIVED: &str = r"^(?:MISSED|TIMEOUT)\s+(.+?):(\d+)(?::\d+)?:\s*(.+)$";

// Proof the run actually happened. cargo-mutants always prints how many mutants it found
// and tested; a command that emits no such summary (e.g. `true`) has not run a mutation
// pass, so a zero-survivor result from it is not evidence of anything. A repo can override
// for another tool via `completionPattern`.
pub const CARGO_MUTANTS_COMPLETION: &str =
    r"(?m)^\s*(?:Found\s+\d+\s+mutants?|\d+\s+mutants?\s+tested)";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Survivor {
    pub file: String,
    pub line: u64,
    pub mutation: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportItem {
    pub file: String,
    pub line: u64,
    pub mutation: String,
    pub instruction: String,
}

pub fn parse_survivors(output: &str, pattern: &str) -> Vec<Survivor> {
    let re = match RegexBuilder::new(pattern).multi_line(true).build() {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let mut survivors = vec![];
    for caps in re.captures_iter(output) {
        // Require the file and line groups; a pattern without them yields nothing
        // rather than garbage survivors.
        let (Some(file), Some(line)) = (caps.get(1), caps.get(2)) else {
            continue;
        };
        survivors.push(Survivor {
            file: file.as_str().to_string(),
            line: line.as_str().parse().unwrap_or(0),
            mutation: caps
                .get(3)
                .map(|m| m.as_str().trim().to_string())
                .unwrap_or_default(),
        });
    }
    survivors
}

// A waiver is the human escape valve for an equivalent or irrelevant mutant. It must
// carry a reason, so "everything is equivalent" is not a silent way to pass.
pub fn validate_waivers(waivers: &[Waiver]) -> Vec<String> {
    let mut failures = vec![];
    for (i, w) in waivers.iter().enumerate() {
        if w.file.is_none() {
            failures.push(format!("waiver[{i}]: missing \"file\""));
        }
        if w.line.is_none() && w.mutation.as_deref().unwrap_or("").trim().is_empty() {
            // An empty mutation substring matches every survivor in the file, so it is not
            // a matcher at all — require a line or a specific fragment.
            failures.push(format!(
                "waiver[{i}]: needs \"line\" or a non-empty \"mutation\" substring to match a specific survivor"
            ));
        }
        if w.reason.as_deref().unwrap_or("").trim().is_empty() {
            failures.push(format!(
                "waiver[{i}]: missing \"reason\" — an equivalent/irrelevant mutant must say why it is waived"
            ));
        }
    }
    failures
}

fn waived(survivor: &Survivor, waivers: &[Waiver]) -> bool {
    waivers.iter().any(|w| {
        w.file.as_deref() == Some(survivor.file.as_str())
            && w.line.is_none_or(|l| l == survivor.line)
            && w.mutation
                .as_ref()
                .is_none_or(|m| survivor.mutation.contains(m.as_str()))
    })
}

pub fn apply_waivers(
    survivors: Vec<Survivor>,
    waivers: &[Waiver],
) -> (Vec<Survivor>, Vec<Survivor>) {
    let mut live = vec![];
    let mut dismissed = vec![];
    for s in survivors {
        if waived(&s, waivers) {
            dismissed.push(s);
        } else {
            live.push(s);
        }
    }
    (live, dismissed)
}

// Tiered verdict: block when a live survivor sits in high-risk code, advisory elsewhere
// so the inner loop stays fast. A change with no live survivors on its diff passes.
pub fn classify(live_count: usize, is_high_risk: bool) -> &'static str {
    if live_count == 0 {
        "pass"
    } else if is_high_risk {
        "block"
    } else {
        "advisory"
    }
}

// Each survivor becomes the exact next test to write — the fault-first feedback that
// closes the loop when handed back to the authoring agent.
pub fn survivor_report(live: &[Survivor]) -> Vec<ReportItem> {
    live.iter()
        .map(|s| ReportItem {
            file: s.file.clone(),
            line: s.line,
            mutation: s.mutation.clone(),
            instruction: format!(
                "At {}:{}, the mutation \"{}\" survived — no test fails when it is applied. Write a test that fails under this mutation.",
                s.file, s.line, s.mutation
            ),
        })
        .collect()
}

pub struct HardenResult {
    pub verdict: String,
    pub error: Option<String>,
    pub waiver_failures: Vec<String>,
    pub report: Vec<ReportItem>,
}

fn blocked(error: &str) -> HardenResult {
    HardenResult {
        verdict: "block".into(),
        error: Some(error.to_string()),
        waiver_failures: vec![],
        report: vec![],
    }
}

// Orchestrate one harden run: execute the diff-scoped mutation command, parse
// survivors, apply waivers, classify by risk. `exec` and `is_high_risk` are injected so
// the core is testable without git or a real mutation run.
pub fn run_harden(
    recipe: &MutationRecipe,
    cwd: &Path,
    waivers: &[Waiver],
    is_high_risk: bool,
    exec: &dyn Exec,
) -> HardenResult {
    let waiver_failures = validate_waivers(waivers);
    if !waiver_failures.is_empty() {
        return HardenResult {
            verdict: "block".into(),
            error: Some("invalid waivers".into()),
            waiver_failures,
            report: vec![],
        };
    }

    let timeout = Duration::from_secs(recipe.timeout_sec.unwrap_or(1800));
    let memory_mb = recipe.memory_mb.unwrap_or(DEFAULT_MEMORY_MB);
    let Some(memory_bytes) = memory_mb.checked_mul(MIB).filter(|bytes| *bytes > 0) else {
        return blocked("memoryMb must be a positive MiB value");
    };
    let out = exec.run_limited(&recipe.cmd, cwd, timeout, memory_bytes);
    // A tool crash or timeout is a hard fail: an unfinished mutation run cannot certify
    // anything, so it must never read as "no survivors".
    if out.timed_out {
        return blocked("mutation run timed out");
    }
    if out.memory_exceeded {
        return blocked(&format!(
            "mutation run exceeded its {memory_mb} MiB memory limit"
        ));
    }
    if out.output_exceeded {
        return blocked("mutation run produced runaway output and was killed to protect the disk");
    }

    let pattern = recipe
        .survivor_pattern
        .as_deref()
        .unwrap_or(CARGO_MUTANTS_SURVIVED);
    let survivors = parse_survivors(&out.output, pattern);

    // With no parseable survivors, trust the tool's own exit code: cargo-mutants exits
    // non-zero when a mutant is missed, so a non-zero exit with nothing parsed means the
    // pattern is wrong, not that the code is clean.
    if survivors.is_empty() && out.code != 0 {
        return blocked(&format!(
            "mutation tool exited {} but no survivors parsed — check survivorPattern",
            out.code
        ));
    }

    // Proof the run happened: either the tool printed its completion summary, or it named
    // survivors (which is itself evidence a pass ran). A command that did neither and exited
    // 0 (e.g. `true`) certifies nothing, so its zero-survivor result must not read as clean.
    let completion = recipe
        .completion_pattern
        .as_deref()
        .unwrap_or(CARGO_MUTANTS_COMPLETION);
    let ran = !survivors.is_empty()
        || RegexBuilder::new(completion)
            .multi_line(true)
            .build()
            .map(|re| re.is_match(&out.output))
            .unwrap_or(false);
    if !ran {
        return blocked(
            "no evidence the mutation run completed — the command produced no mutation summary (e.g. \"N mutants tested\"); a zero-survivor result cannot be trusted. Check the mutation cmd and completionPattern.",
        );
    }

    let (live, _dismissed) = apply_waivers(survivors, waivers);
    let verdict = classify(live.len(), is_high_risk);
    let report = survivor_report(&live);
    HardenResult {
        verdict: verdict.to_string(),
        error: None,
        waiver_failures: vec![],
        report,
    }
}

#[cfg(test)]
#[path = "mutation_tests.rs"]
mod tests;
