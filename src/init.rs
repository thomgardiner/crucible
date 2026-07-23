//! `crucible init`: scaffold `.crucible/` into a repo so a team can adopt the framework
//! without hand-copying config. Idempotent — an existing file is left alone (reported as
//! skipped) unless force is set, so re-running never clobbers real config. The starters
//! are valid JSON that parse and load; the TODO markers are the checklist `crucible
//! check` and the next-steps output walk through.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

// One entry per file init writes under `.crucible/`, in write order.
pub fn starters() -> Vec<(&'static str, Value)> {
    vec![
        (
            "adapter.json",
            json!({
                "repo": "TODO-your-repo-name",
                "charter": ".crucible/charter.json",
                "approvals": ".crucible/approvals.json",
                "gateRunner": {
                    "command": "TODO how the required per-change gate is run, e.g. 'make verify' or 'grove verify change'",
                    "file": "TODO/path/to/your/gate-runner-script",
                    "checkerPattern": "node (checks/check-[a-z-]+\\.mjs)"
                },
                "changeToUnits": "TODO (optional) how changed files map to build units, e.g. 'tools/test-impact.mjs'",
                "highRiskUnits": [],
                "prePush": ".githooks/pre-push",
                "pinnedConfig": [".crucible/adapter.json", ".crucible/acceptance.json", ".crucible/mutation.json"]
            }),
        ),
        (
            "charter.json",
            json!({
                "_note": "The Gate Ledger: one row per correctness gate. Every T1 gate must be wired in the gateRunner file (adapter.gateRunner.file). Replace the placeholder below with your real gates and pin each with `crucible approve <id> --by <reviewer>`.",
                "gates": [{
                    "id": "example-gate",
                    "rule": "TODO the invariant this gate enforces",
                    "tier": "T3",
                    "reason": "placeholder from `crucible init`; replace with a real gate wired at T1, or delete this row"
                }]
            }),
        ),
        (
            "acceptance.json",
            json!({
                "_note": "The reality recipe: build, boot, and drive the real app. boot.oracle.stdoutMatch and a non-empty drive[] are required — without them a run cannot prove the app came up.",
                "repo": "TODO-your-repo-name",
                "build": { "cmd": "TODO build the app, e.g. 'cargo build' or 'npm run build'" },
                "boot": {
                    "cmd": "TODO launch the app so it initializes (DB, migrations, config)",
                    "oracle": { "stdoutMatch": "TODO a string only a healthy boot prints", "stdoutForbid": "panic|FATAL|migration failed" }
                },
                "drive": [
                    { "name": "smoke", "cmd": "TODO drive one real user flow end to end", "oracle": { "stdoutMatch": "TODO success marker" } }
                ],
                "trust": {
                    "testRoots": ["src"],
                    "testPattern": "(\\.test\\.[tj]sx?$|_tests?\\.rs$)",
                    "mockMarkers": ["wiremock", "MockServer", "mockIPC("]
                }
            }),
        ),
        (
            "mutation.json",
            json!({
                "_note": "Diff-scoped mutation. cmd runs the mutation tool over changed code and emits survivor lines (MISSED/TIMEOUT); base is what the diff is taken against. The entire mutation process tree is killed if it exceeds memoryMb. Surviving mutants are written to .crucible/survivors.json as the next tests to write.",
                "cmd": "TODO diff-scoped mutation command, e.g. 'cargo mutants --in-diff origin/main' or 'npx stryker run'",
                "base": "origin/main",
                "memoryMb": 2048,
                "survivorPattern": "^(?:MISSED|TIMEOUT)\\s+([^\\s:]+):(\\d+)(?::\\d+)?:\\s*(.+)$"
            }),
        ),
        (
            "coverage.json",
            json!({
                "_note": "The coverage floor: a command that emits LCOV (to lcovPath, or stdout). Diff-scoped. Reports changed functions no test ever calls — reachability, the floor under the mutation gate. Add \"memoryMb\": <N> to cap the coverage build's process-tree memory (a runaway is killed).",
                "cmd": "TODO a command that emits LCOV, e.g. 'cargo llvm-cov --workspace --lcov --output-path target/lcov.info' or 'npx c8 --reporter=lcovonly npm test'",
                "base": "origin/main",
                "lcovPath": "target/lcov.info"
            }),
        ),
        (
            "flake.json",
            json!({
                "_note": "The determinism check: a test command run N times to catch nondeterminism. failPattern's group 1 is a failed test's name; without it, only exit codes are compared. A flaky green is a false green. Add \"memoryMb\": <N> to cap each run's process-tree memory (a runaway is killed).",
                "cmd": "TODO the test command to run repeatedly, e.g. 'cargo nextest run' or 'npm test'",
                "runs": 3,
                "failPattern": "FAIL(?:ED)?\\s+([\\w:./-]+)"
            }),
        ),
        ("approvals.json", json!([])),
    ]
}

pub struct ScaffoldResult {
    pub dir: PathBuf,
    pub written: Vec<String>,
    pub skipped: Vec<String>,
}

/// Starter pre-push hook: runs the honesty gate so independence is a verified fact.
pub const PRE_PUSH_HOOK: &str = r#"#!/usr/bin/env sh
# Crucible independence layer — do not remove `crucible check`.
# Approvals must be committed separately from the config they bless.
set -e
if command -v crucible >/dev/null 2>&1; then
  crucible check
else
  echo "crucible: not on PATH — install the CLI before pushing" >&2
  exit 1
fi
"#;

// Scaffold `.crucible/` under repo_root. Returns which files were written vs skipped so
// the caller can report accurately and exit non-zero if nothing was written.
pub fn scaffold(repo_root: &Path, force: bool) -> Result<ScaffoldResult> {
    let dir = repo_root.join(".crucible");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut written = vec![];
    let mut skipped = vec![];
    for (name, body) in starters() {
        let path = dir.join(name);
        if path.exists() && !force {
            skipped.push(name.to_string());
            continue;
        }
        let text = serde_json::to_string_pretty(&body)? + "\n";
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        written.push(name.to_string());
    }

    // Load-bearing pre-push: adapter.prePush points here; check fails if it is missing
    // or does not run `crucible check`.
    let hooks = repo_root.join(".githooks");
    std::fs::create_dir_all(&hooks).with_context(|| format!("creating {}", hooks.display()))?;
    let pre_push = hooks.join("pre-push");
    if pre_push.exists() && !force {
        skipped.push(".githooks/pre-push".into());
    } else {
        std::fs::write(&pre_push, PRE_PUSH_HOOK)
            .with_context(|| format!("writing {}", pre_push.display()))?;
        // Best-effort executable bit (no-op / ignored on some platforms).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&pre_push, std::fs::Permissions::from_mode(0o755));
        }
        written.push(".githooks/pre-push".into());
    }

    Ok(ScaffoldResult {
        dir,
        written,
        skipped,
    })
}

#[cfg(test)]
#[path = "init_tests.rs"]
mod tests;
