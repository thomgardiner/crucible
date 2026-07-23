//! The reality arm: deterministically test whether a repo ACTUALLY works rather than
//! trusting a green unit suite. It runs the real thing through fixed phases — build the
//! shippable artifact, boot it, drive the real critical paths against real oracles — and
//! audits how much of the "green" suite only mocks the seam where an app crashes.

use crate::config::{Recipe, Step, TrustCfg};
use crate::proc::Exec;
use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;
use walkdir::WalkDir;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepResult {
    pub name: String,
    pub cmd: String,
    pub ok: bool,
    pub code: i32,
    pub timed_out: bool,
    pub output: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustReport {
    pub total: usize,
    pub mocked: usize,
    pub real_boundary: usize,
    pub by_marker: BTreeMap<String, usize>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub repo: String,
    pub build: Option<StepResult>,
    pub boot: Option<StepResult>,
    pub drive: Vec<StepResult>,
    pub trust: Option<TrustReport>,
    pub verdict: String,
    pub summary: String,
}

fn tail_str(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() > n {
        let t: String = chars[chars.len() - n..].iter().collect();
        format!("…{t}")
    } else {
        s.to_string()
    }
}

fn run_step(
    exec: &dyn Exec,
    repo_root: &Path,
    name: &str,
    step: &Step,
    memory_bytes: Option<u64>,
) -> StepResult {
    let cwd = match &step.cwd {
        Some(c) => repo_root.join(c),
        None => repo_root.to_path_buf(),
    };
    let timeout = Duration::from_secs(step.timeout_sec.unwrap_or(300));
    // Cap every build/boot/drive step's process tree so a runaway app cannot OOM the box.
    let out = match memory_bytes {
        Some(bytes) => exec.run_limited(&step.cmd, &cwd, timeout, bytes),
        None => exec.run(&step.cmd, &cwd, timeout),
    };
    let mut ok = out.code == 0;
    // An exit-0 command can still have failed its real check; a stdoutMatch oracle is
    // the real signal for a launch that returns 0 but never became healthy.
    if ok
        && let Some(oracle) = &step.oracle
        && let Some(m) = &oracle.stdout_match
    {
        ok = Regex::new(m)
            .map(|re| re.is_match(&out.output))
            .unwrap_or(false);
    }
    if let Some(oracle) = &step.oracle
        && let Some(forbid) = &oracle.stdout_forbid
        && Regex::new(forbid)
            .map(|re| re.is_match(&out.output))
            .unwrap_or(false)
    {
        ok = false;
    }
    StepResult {
        name: name.to_string(),
        cmd: step.cmd.clone(),
        ok,
        code: out.code,
        timed_out: out.timed_out,
        output: tail_str(&out.output, 4000),
    }
}

// Static trust audit: how many test files only exercise the system through a mocked
// boundary (the seam a real launch crosses and a mock never does). This is what the
// green count hides.
pub fn trust_audit(repo_root: &Path, cfg: &TrustCfg) -> TrustReport {
    let pattern = Regex::new(&cfg.test_pattern).ok();
    let mut files: Vec<String> = vec![];
    for root in &cfg.test_roots {
        let base = repo_root.join(root);
        for entry in WalkDir::new(&base)
            .into_iter()
            .filter_entry(|e| {
                !matches!(
                    e.file_name().to_str(),
                    Some("node_modules") | Some("target") | Some(".git")
                )
            })
            .flatten()
        {
            if entry.file_type().is_file() {
                let p = entry.path().to_string_lossy().to_string();
                if pattern.as_ref().map(|re| re.is_match(&p)).unwrap_or(false) {
                    files.push(p);
                }
            }
        }
    }
    let mut by_marker: BTreeMap<String, usize> =
        cfg.mock_markers.iter().map(|m| (m.clone(), 0)).collect();
    let mut mocked = 0;
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap_or_default();
        let mut hit = false;
        for m in &cfg.mock_markers {
            if src.contains(m) {
                *by_marker.get_mut(m).unwrap() += 1;
                hit = true;
            }
        }
        if hit {
            mocked += 1;
        }
    }
    TrustReport {
        total: files.len(),
        mocked,
        real_boundary: files.len() - mocked,
        by_marker,
    }
}

fn broken(repo: &str, summary: &str, trust: Option<TrustReport>) -> Report {
    Report {
        repo: repo.to_string(),
        build: None,
        boot: None,
        drive: vec![],
        trust,
        verdict: "BROKEN".into(),
        summary: summary.to_string(),
    }
}

const BOOT_MISSING: &str =
    "no readiness oracle (a non-empty oracle.stdoutMatch) — exit 0 is not proof the app launched";
const DRIVE_MISSING: &str = "no oracle — exit 0 is not proof a real flow ran; add a non-empty stdoutMatch that consumes real output";

// The reason a step's oracle cannot prove the step did real work, or None if it can. A
// positive stdoutMatch is required (exit 0 alone proves nothing, and a stdoutForbid alone
// passes on empty output — "did not print ERROR" is not evidence a flow ran). It must
// compile, and it must not match the empty string (".*", ".?" pass on no output at all).
// A stdoutForbid, if present, must also compile, so an invalid pattern can never silently
// disable the negative check.
fn oracle_defect(step: &Step, missing: &str) -> Option<String> {
    let oracle = step.oracle.as_ref();
    let m = oracle
        .and_then(|o| o.stdout_match.as_deref())
        .filter(|m| !m.is_empty());
    let Some(m) = m else {
        return Some(missing.to_string());
    };
    let re = match Regex::new(m) {
        Ok(re) => re,
        Err(e) => return Some(format!("oracle.stdoutMatch is not a valid regex: {e}")),
    };
    if re.is_match("") {
        return Some(format!(
            "oracle.stdoutMatch \"{m}\" matches empty output, so exit 0 with no output would pass — use a pattern that only matches real ready/success output"
        ));
    }
    if let Some(forbid) = oracle
        .and_then(|o| o.stdout_forbid.as_deref())
        .filter(|f| !f.is_empty())
        && let Err(e) = Regex::new(forbid)
    {
        return Some(format!("oracle.stdoutForbid is not a valid regex: {e}"));
    }
    None
}

// Run the full reality arm. Phases are ordered by prerequisite: a repo that does not
// build cannot boot, and one that does not boot cannot be driven. The trust audit is
// static and always runs, because the gap it measures is the whole point.

pub fn run_crucible(
    repo_root: &Path,
    recipe: &Recipe,
    exec: &dyn Exec,
    memory_bytes: Option<u64>,
) -> Report {
    let repo = recipe.repo.clone().unwrap_or_else(|| "repo".into());

    // A recipe that cannot prove readiness or drives nothing must never yield RUNS:
    // exit 0 is not proof the app came up, and build+boot with no critical path
    // exercises no application behavior at all. Each driven step's oracle must be a real
    // signal — present, compilable, and not one that passes on empty output.
    let boot_defect = match recipe.boot.as_ref() {
        Some(b) => oracle_defect(b, BOOT_MISSING),
        None => Some(BOOT_MISSING.to_string()),
    };
    if let Some(reason) = boot_defect {
        return broken(&repo, &format!("boot step: {reason}"), None);
    }
    if recipe.drive.is_empty() {
        return broken(
            &repo,
            "no critical paths to drive — build and boot alone prove nothing about behavior",
            None,
        );
    }
    for d in &recipe.drive {
        if let Some(reason) = oracle_defect(d, DRIVE_MISSING) {
            return broken(
                &repo,
                &format!(
                    "drive step '{}': {reason}",
                    d.name.clone().unwrap_or_else(|| "?".into())
                ),
                None,
            );
        }
    }
    // The commands themselves must exist; an empty command "succeeds" by doing nothing.
    for (label, step) in [
        ("build", recipe.build.as_ref()),
        ("boot", recipe.boot.as_ref()),
    ] {
        if let Some(s) = step
            && s.cmd.trim().is_empty()
        {
            return broken(&repo, &format!("{label} step has an empty command"), None);
        }
    }
    if let Some(bad) = recipe.drive.iter().find(|d| d.cmd.trim().is_empty()) {
        return broken(
            &repo,
            &format!(
                "drive step '{}' has an empty command",
                bad.name.clone().unwrap_or_else(|| "?".into())
            ),
            None,
        );
    }

    let Some(build_step) = &recipe.build else {
        return broken(&repo, "no build step in the recipe", None);
    };
    let build = run_step(exec, repo_root, "build", build_step, memory_bytes);
    let trust = recipe.trust.as_ref().map(|t| trust_audit(repo_root, t));

    if !build.ok {
        return Report {
            repo,
            build: Some(build),
            boot: None,
            drive: vec![],
            trust,
            verdict: "BROKEN".into(),
            summary: "does not build".into(),
        };
    }

    let boot = run_step(
        exec,
        repo_root,
        "boot",
        recipe.boot.as_ref().unwrap(),
        memory_bytes,
    );
    if !boot.ok {
        // Say what actually happened: exit 0 without the readiness oracle is not a crash,
        // it is a launch that never proved it came up (Codex round 5: a wrong reason in
        // the verdict is its own kind of false report).
        let summary = if boot.timed_out {
            "app did not reach ready within timeout"
        } else if boot.code == 0 {
            "app exited 0 but never printed its readiness oracle"
        } else {
            "app crashed on launch"
        };
        return Report {
            repo,
            build: Some(build),
            boot: Some(boot),
            drive: vec![],
            trust,
            verdict: "BROKEN".into(),
            summary: summary.into(),
        };
    }

    let mut drive = vec![];
    for (i, d) in recipe.drive.iter().enumerate() {
        let name = d.name.clone().unwrap_or_else(|| format!("drive[{i}]"));
        drive.push(run_step(exec, repo_root, &name, d, memory_bytes));
    }
    let drive_ok = drive.iter().all(|d| d.ok);
    let failed: Vec<String> = drive
        .iter()
        .filter(|d| !d.ok)
        .map(|d| d.name.clone())
        .collect();
    let summary = if drive_ok {
        "builds, boots, and drives every real critical path".to_string()
    } else {
        format!(
            "builds and boots, but real flow(s) failed: {}",
            failed.join(", ")
        )
    };
    Report {
        repo,
        build: Some(build),
        boot: Some(boot),
        drive,
        trust,
        verdict: if drive_ok {
            "RUNS".into()
        } else {
            "BROKEN".into()
        },
        summary,
    }
}

// Human-readable, deterministic report. Same inputs produce the same text.
pub fn format_report(r: &Report) -> String {
    let line = |label: &str, res: &StepResult| -> String {
        let flag = if res.ok { "PASS" } else { "FAIL" };
        let t = if res.timed_out { " (timed out)" } else { "" };
        let code = if res.ok {
            String::new()
        } else {
            format!(" (exit {})", res.code)
        };
        format!("  {flag}  {label}{t}{code}")
    };
    let verdict = if r.verdict == "RUNS" {
        "the app actually runs".to_string()
    } else {
        format!("BROKEN — {}", r.summary)
    };
    let mut out = vec![
        format!("CRUCIBLE — {}", r.repo),
        format!("VERDICT: {verdict}"),
        String::new(),
    ];
    if let Some(b) = &r.build {
        out.push(line("build the real artifact", b));
    }
    if let Some(b) = &r.boot {
        out.push(line("boot the real app to ready", b));
    }
    for d in &r.drive {
        out.push(line(&format!("drive: {}", d.name), d));
    }
    if let Some(t) = &r.trust {
        let pct = (t.mocked * 100).checked_div(t.total).unwrap_or(0);
        out.push(String::new());
        out.push(format!(
            "TRUST: {}/{} test files ({pct}%) only cross a mocked boundary",
            t.mocked, t.total
        ));
        out.push("  (green there says nothing about whether the real app boots — the phases above are the real coverage)".into());
    }
    if r.verdict != "RUNS" {
        // First failing phase in prerequisite order: a failed boot, else a failed drive,
        // else the failed build. (Drive steps only run after a passing boot, so there is
        // no boot-absent-with-drives case.)
        let crashed = r
            .boot
            .as_ref()
            .filter(|b| !b.ok)
            .or_else(|| r.drive.iter().find(|d| !d.ok))
            .or_else(|| r.build.as_ref().filter(|b| !b.ok));
        if let Some(c) = crashed {
            out.push(String::new());
            out.push(format!("--- {} output (tail) ---", c.name));
            out.push(c.output.clone());
        }
    }
    out.join("\n")
}

#[cfg(test)]
#[path = "reality_tests.rs"]
mod tests;
