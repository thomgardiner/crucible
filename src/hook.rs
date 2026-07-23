//! Agent-loop hooks. The plugin wires TUI hook events (Claude Code, Codex) to
//! `crucible hook <event>`, so the logic lives in this binary and stays identical
//! across any TUI that speaks the hook JSON protocol. The load-bearing one is Stop:
//! when an agent tries to finish an adopted repo with uncommitted work and no recent
//! verification, it is nudged to run `crucible run`/`harden` first. This is in-loop
//! pressure, not a sealed guarantee — CI and pre-push remain the hard backstop.

use crate::hash::{content_fingerprint, sha256_hex};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// A verification is "recent" within this window, so a just-verified agent is not nagged.
const RECEIPT_MAX_AGE_SECS: u64 = 1800;

/// First line of every receipt this binary writes. Casual `echo` forgeries without it
/// do not clear the Stop nudge. Not a secret — raises the bar, does not seal it.
const RECEIPT_MAGIC: &str = "CRUCIBLE-RECEIPT-v1";

pub struct HookResult {
    pub stdout: String,
    pub exit: u8,
}

fn empty() -> HookResult {
    HookResult {
        stdout: String::new(),
        exit: 0,
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Arms whose success means the *change* was verified for Stop-nudge purposes.
// `check` is gate honesty only — an agent must not clear the nudge with check alone
// while never running tests or mutation. `cover` is "was it executed", not "tests bite".
// `flake` is determinism only. So only run (app real) and harden (tests bite) count.
const VERIFYING_ARMS: [&str; 2] = ["run", "harden"];

// Per-repo, per-arm receipts in the OS temp dir (keyed by repo path), written whenever
// the agent verifies. Kept out of the repo so they never churn the working tree. Each
// receipt records WHICH arm verified and a fingerprint of the worktree it verified, so
// a receipt from one arm cannot satisfy another's requirement and edits made after the
// verification invalidate it.
pub fn receipt_path(repo: &Path, arm: &str) -> PathBuf {
    let key = sha256_hex(repo.to_string_lossy().as_bytes());
    std::env::temp_dir()
        .join("crucible-receipts")
        .join(format!("{key}.{arm}.receipt"))
}

// Full stream for normal source; head+tail for multi-MB blobs so Stop stays bounded.
const FINGERPRINT_FULL_MAX: u64 = 8 * 1024 * 1024;

// Fingerprint of the uncommitted state: every dirty/staged path + content digest
// (streamed, constant memory) + HEAD. Empty only when git cannot answer.
fn tree_fingerprint(repo: &Path) -> String {
    let git = |args: &[&str]| -> Option<Vec<u8>> {
        let out = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    };
    // Modified-tracked + untracked, PLUS staged (index-vs-HEAD) changes — a post-verify
    // edit followed by `git add` matches the worktree to the index, so ls-files -m would
    // miss it; --cached catches it (Codex resource review).
    let (Some(worktree), Some(staged)) = (
        git(&["ls-files", "-m", "-o", "--exclude-standard", "-z"]),
        git(&["diff", "--cached", "--name-only", "-z"]),
    ) else {
        return String::new();
    };
    let mut input = git(&["rev-parse", "HEAD"]).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    for raw in worktree
        .split(|b| *b == 0)
        .chain(staged.split(|b| *b == 0))
        .filter(|p| !p.is_empty())
    {
        if !seen.insert(raw.to_vec()) {
            continue; // a file can be both staged and worktree-modified; count it once
        }
        input.extend_from_slice(raw);
        input.push(0);
        let path = repo.join(String::from_utf8_lossy(raw).as_ref());
        if let Ok(meta) = std::fs::metadata(&path) {
            input.extend_from_slice(format!("{}\x00", meta.len()).as_bytes());
            // Content digest (full when small; head+tail when huge) so post-sample edits
            // still invalidate without streaming multi-GB dirty artifacts on every Stop.
            if let Ok(digest) = content_fingerprint(&path, FINGERPRINT_FULL_MAX) {
                input.extend_from_slice(digest.as_bytes());
            }
        }
        input.push(b'\n');
    }
    sha256_hex(&input)
}

pub fn write_receipt(repo: &Path, arm: &str) {
    let p = receipt_path(repo, arm);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let body = format!(
        "{RECEIPT_MAGIC}\n{arm}\n{}\n{}\n",
        now_secs(),
        tree_fingerprint(repo)
    );
    let _ = std::fs::write(&p, body);
}

fn receipt_fresh(repo: &Path, arm: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(receipt_path(repo, arm)) else {
        return false;
    };
    let mut lines = text.lines();
    // Magic + arm bind the file to this binary's writer and the arm that ran.
    if lines.next().map(str::trim) != Some(RECEIPT_MAGIC) {
        return false;
    }
    if lines.next().map(str::trim) != Some(arm) {
        return false;
    }
    let Some(Ok(then)) = lines.next().map(|l| l.trim().parse::<u64>()) else {
        return false;
    };
    if now_secs().saturating_sub(then) > RECEIPT_MAX_AGE_SECS {
        return false;
    }
    // The worktree must still be the one that was verified: edits after the receipt
    // reopen the question.
    let recorded = lines.next().unwrap_or("").trim();
    recorded == tree_fingerprint(repo)
}

// True when any change-verifying arm has a fresh receipt for the current worktree.
fn verified_recently(repo: &Path) -> bool {
    VERIFYING_ARMS.iter().any(|arm| receipt_fresh(repo, arm))
}

fn adopted(repo: &Path) -> bool {
    repo.join(".crucible").is_dir()
}

fn nudge_disabled() -> bool {
    matches!(
        std::env::var("CRUCIBLE_NO_NUDGE").as_deref(),
        Ok("1") | Ok("true")
    )
}

// True when there is real work to verify, or when git cannot answer in an *adopted*
// repo (fail closed: do not silently skip the nudge because PATH has no git).
fn has_uncommitted_changes(repo: &Path) -> bool {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo)
        .output();
    match out {
        Ok(o) if o.status.success() => !o.stdout.iter().all(|b| b.is_ascii_whitespace()),
        // Adopted + no git → assume dirty so Stop still pressures verification.
        _ => adopted(repo),
    }
}

// The pure Stop decision, isolated so every combination is testable without git or a
// real session. Nudge only when: the repo adopted Crucible, the agent is finishing for
// the first time this turn, nudging is enabled, there is uncommitted work, and no recent
// verification exists.
pub fn should_block_stop(
    adopted: bool,
    stop_hook_active: bool,
    disabled: bool,
    has_changes: bool,
    fresh: bool,
) -> bool {
    adopted && !stop_hook_active && !disabled && has_changes && !fresh
}

const STOP_REASON: &str = "This repo uses Crucible and you are finishing with uncommitted changes, but no `crucible run`/`harden` ran successfully on this worktree recently. `crucible check` alone is not enough — it only audits gates, it does not prove tests bite or the app boots. Run `crucible harden` (do the tests constrain behavior?) and/or `crucible run` (does it boot and drive?), fix what they surface, then stop. If verification genuinely does not apply to this change, say why.";

const SESSION_CONTEXT: &str = "Crucible is active in this repo. Before reporting a change as tested or done: `crucible harden` (tests bite), `crucible run` (app boots/drives), plus `crucible check` (gates honest) and `crucible test-smells` when you touched tests. A green unit suite or check-only receipt is not proof.";

fn stop(repo: &Path, stop_hook_active: bool) -> HookResult {
    let block = should_block_stop(
        adopted(repo),
        stop_hook_active,
        nudge_disabled(),
        has_uncommitted_changes(repo),
        verified_recently(repo),
    );
    if block {
        HookResult {
            stdout: json!({ "decision": "block", "reason": STOP_REASON }).to_string(),
            exit: 0,
        }
    } else {
        empty()
    }
}

fn session_start(repo: &Path) -> HookResult {
    if !adopted(repo) || nudge_disabled() {
        return empty();
    }
    let out = json!({
        "hookSpecificOutput": { "hookEventName": "SessionStart", "additionalContext": SESSION_CONTEXT }
    });
    HookResult {
        stdout: out.to_string(),
        exit: 0,
    }
}

// Dispatch a hook event given the raw stdin payload the TUI provides. `cwd` and
// `stop_hook_active` are read from it. Unparsable payload is a safe no-op — never
// fall back to process cwd for Stop, or adopting the product repo itself would
// spuriously block every garbage-input path (including our own unit tests).
pub fn run_hook(event: &str, input: &str) -> HookResult {
    let Ok(v) = serde_json::from_str::<Value>(input) else {
        return empty();
    };
    let cwd = v
        .get("cwd")
        .and_then(|c| c.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    match event {
        "session-start" => session_start(&cwd),
        "stop" => stop(
            &cwd,
            v.get("stop_hook_active")
                .and_then(|b| b.as_bool())
                .unwrap_or(false),
        ),
        _ => empty(),
    }
}

#[cfg(test)]
#[path = "hook_tests.rs"]
mod tests;
