//! `crucible init`: scaffold `.crucible/` into a repo so a team can adopt the framework
//! without hand-copying config. Idempotent — an existing file is left alone (reported as
//! skipped) unless force is set, so re-running never clobbers real config. Starters are
//! valid JSON that parse and load; a minimal smoke gate is wired so `doctor` can pass
//! after approve, while acceptance/mutation recipes keep TODOs for real app commands.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Smoke checker path relative to the repo root.
pub const SMOKE_CHECKER: &str = "checks/check-smoke.sh";
/// Gate runner path relative to the repo root.
pub const GATE_RUNNER: &str = "scripts/verify.sh";

pub const SMOKE_CHECKER_BODY: &str = r#"#!/usr/bin/env sh
# Placeholder T1 gate from `crucible init`. Replace with a real invariant,
# or delete this file and the matching charter row once you add real gates.
set -e
exit 0
"#;

pub const GATE_RUNNER_BODY: &str = r#"#!/usr/bin/env sh
# Required per-change lane. Every T1 checker is invoked here.
set -e
cd "$(dirname "$0")/.."
sh checks/check-smoke.sh
"#;

// One entry per file init writes under `.crucible/`, in write order.
pub fn starters(repo_name: &str) -> Vec<(&'static str, Value)> {
    vec![
        (
            "adapter.json",
            json!({
                "repo": repo_name,
                "charter": ".crucible/charter.json",
                "approvals": ".crucible/approvals.json",
                "gateRunner": {
                    "command": "sh scripts/verify.sh",
                    "file": "scripts/verify.sh",
                    "checkerPattern": "sh (checks/check-[a-z-]+\\.sh)"
                },
                "highRiskUnits": [],
                "prePush": ".githooks/pre-push",
                "pinnedConfig": [
                    ".crucible/adapter.json",
                    ".crucible/acceptance.json",
                    ".crucible/mutation.json",
                    ".crucible/coverage.json",
                    ".crucible/flake.json"
                ]
            }),
        ),
        (
            "charter.json",
            json!({
                "_note": "Gate ledger. Every T1 gate must be wired in adapter.gateRunner.file. Replace the smoke placeholder with real gates, or delete it.",
                "gates": [{
                    "id": "smoke",
                    "rule": "Placeholder smoke gate from crucible init — replace with a real invariant",
                    "tier": "T1",
                    "checker": "checks/check-smoke.sh",
                    "blockingCondition": "always"
                }]
            }),
        ),
        (
            "acceptance.json",
            json!({
                "_note": "Reality recipe: build, boot, and drive the real app. Required for `crucible run`.",
                "repo": repo_name,
                "build": { "cmd": "TODO e.g. cargo build or npm run build" },
                "boot": {
                    "cmd": "TODO e.g. cargo run -- --help  (must print a ready marker)",
                    "oracle": { "stdoutMatch": "TODO string only a healthy boot prints", "stdoutForbid": "panic|FATAL" }
                },
                "drive": [
                    { "name": "smoke", "cmd": "TODO one real user-flow command", "oracle": { "stdoutMatch": "TODO success marker" } }
                ],
                "trust": {
                    "testRoots": ["src", "tests"],
                    "testPattern": "(\\.test\\.[tj]sx?$|_tests?\\.rs$)",
                    "mockMarkers": ["wiremock", "MockServer", "mockIPC("]
                }
            }),
        ),
        (
            "mutation.json",
            json!({
                "_note": "Diff-scoped mutation for `crucible harden`. Requires a mutation tool on PATH.",
                "cmd": "TODO e.g. cargo mutants --in-diff HEAD --timeout 120 -j 1 --cargo-test-arg=--bins",
                "base": "HEAD",
                "memoryMb": 2048,
                "survivorPattern": "^(?:MISSED|TIMEOUT)\\s+([^\\s:]+):(\\d+)(?::\\d+)?:\\s*(.+)$"
            }),
        ),
        (
            "coverage.json",
            json!({
                "_note": "LCOV emitter for `crucible cover`.",
                "cmd": "TODO e.g. cargo llvm-cov --bins --lcov --output-path target/lcov.info",
                "base": "HEAD",
                "lcovPath": "target/lcov.info"
            }),
        ),
        (
            "flake.json",
            json!({
                "_note": "Repeat the suite for `crucible flake`. Prefer unit tests if integration tests re-enter Crucible.",
                "cmd": "TODO e.g. cargo test -q --bins",
                "runs": 2,
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
    /// Set when init configured `core.hooksPath` for this repo (message for the user).
    pub hooks_path_note: Option<String>,
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
  echo "  https://github.com/thomgardiner/crucible/releases" >&2
  exit 1
fi
"#;

fn repo_name(repo_root: &Path) -> String {
    repo_root
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != "/" && *s != ".")
        .unwrap_or("my-app")
        .to_string()
}

fn write_exec(path: &Path, body: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(())
}

/// If this is a git repo and `core.hooksPath` is unset, point it at `.githooks`.
fn ensure_hooks_path(repo_root: &Path) -> Option<String> {
    let ok = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo_root)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    if !String::from_utf8_lossy(&ok.stdout)
        .trim()
        .eq_ignore_ascii_case("true")
    {
        return None;
    }
    let existing = Command::new("git")
        .args(["config", "--get", "core.hooksPath"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if existing.status.success() {
        let v = String::from_utf8_lossy(&existing.stdout).trim().to_string();
        if !v.is_empty() {
            return None; // already configured — do not clobber
        }
    }
    let set = Command::new("git")
        .args(["config", "core.hooksPath", ".githooks"])
        .current_dir(repo_root)
        .status()
        .ok()?;
    if set.success() {
        Some("git config core.hooksPath .githooks (local to this repo)".into())
    } else {
        None
    }
}

// Scaffold `.crucible/` under repo_root. Returns which files were written vs skipped so
// the caller can report accurately and exit non-zero if nothing was written.
pub fn scaffold(repo_root: &Path, force: bool) -> Result<ScaffoldResult> {
    let name = repo_name(repo_root);
    let dir = repo_root.join(".crucible");
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut written = vec![];
    let mut skipped = vec![];
    for (fname, body) in starters(&name) {
        let path = dir.join(fname);
        if path.exists() && !force {
            skipped.push(fname.to_string());
            continue;
        }
        let text = serde_json::to_string_pretty(&body)? + "\n";
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        written.push(fname.to_string());
    }

    // Minimal smoke gate so doctor/check can pass after approve without a blank slate.
    for (rel, body) in [
        (SMOKE_CHECKER, SMOKE_CHECKER_BODY),
        (GATE_RUNNER, GATE_RUNNER_BODY),
    ] {
        let path = repo_root.join(rel);
        if path.exists() && !force {
            skipped.push(rel.into());
        } else {
            write_exec(&path, body)?;
            written.push(rel.into());
        }
    }

    // Load-bearing pre-push.
    let hooks = repo_root.join(".githooks");
    std::fs::create_dir_all(&hooks).with_context(|| format!("creating {}", hooks.display()))?;
    let pre_push = hooks.join("pre-push");
    if pre_push.exists() && !force {
        skipped.push(".githooks/pre-push".into());
    } else {
        write_exec(&pre_push, PRE_PUSH_HOOK)?;
        written.push(".githooks/pre-push".into());
    }

    let hooks_path_note = ensure_hooks_path(repo_root);

    Ok(ScaffoldResult {
        dir,
        written,
        skipped,
        hooks_path_note,
    })
}

#[cfg(test)]
#[path = "init_tests.rs"]
mod tests;
