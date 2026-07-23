//! Agent-loop hooks. The plugin wires TUI hook events (Claude Code, Codex) to
//! `crucible hook <event>`, so the logic lives in this binary and stays identical
//! across any TUI that speaks the hook JSON protocol. The load-bearing one is Stop:
//! when an agent tries to finish an adopted repo with uncommitted work and no recent
//! verification, it is nudged to run `crucible run`/`harden` first. This is the
//! enforcement the CLI-in-CI cannot provide inside the agent's loop.

use crate::hash::sha256_hex;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

// A verification is "recent" within this window, so a just-verified agent is not nagged.
const RECEIPT_MAX_AGE_SECS: u64 = 1800;

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

// The arms whose success means the CHANGE was verified. `flake` deliberately does not
// count: determinism says nothing about the change being correct, so a flake receipt
// must never satisfy a nudge asking for run/harden/check (Codex round 3).
const VERIFYING_ARMS: [&str; 4] = ["check", "run", "harden", "cover"];

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

// How many leading bytes of each dirty file feed the fingerprint. Bounded so a multi-GB
// changed file cannot be buffered into memory (Codex resource review): a full `git diff`
// or whole-file read would OOM. A real post-verification edit changes a file's size,
// mtime, or leading bytes, so path + len + mtime + this sample invalidates the receipt.
const FINGERPRINT_SAMPLE: usize = 4096;

// Fingerprint of the uncommitted state: for every modified-tracked and untracked file
// (`git ls-files -m -o -z` gives clean NUL-separated paths, including inside untracked
// directories), a bounded digest of its path, size, mtime, and leading bytes, plus the
// current HEAD so a commit or branch switch also invalidates. Empty when git cannot
// answer, which downgrades the binding to time-only (consistent with
// has_uncommitted_changes failing open without git).
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
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            input.extend_from_slice(format!("{}\x00{mtime}\x00", meta.len()).as_bytes());
            if let Ok(mut f) = std::fs::File::open(&path) {
                let mut sample = vec![0u8; FINGERPRINT_SAMPLE];
                let n = std::io::Read::read(&mut f, &mut sample).unwrap_or(0);
                input.extend_from_slice(&sample[..n]);
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
    let body = format!("{}\n{}", now_secs(), tree_fingerprint(repo));
    let _ = std::fs::write(&p, body);
}

fn receipt_fresh(repo: &Path, arm: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(receipt_path(repo, arm)) else {
        return false;
    };
    let mut lines = text.lines();
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

// True only when there is real work to verify: uncommitted changes in the repo. Fails
// open (no nudge) when git cannot answer, so a broken or absent git never nags.
fn has_uncommitted_changes(repo: &Path) -> bool {
    let out = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo)
        .output();
    match out {
        Ok(o) if o.status.success() => !o.stdout.iter().all(|b| b.is_ascii_whitespace()),
        _ => false,
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

const STOP_REASON: &str = "This repo uses Crucible and you are finishing with uncommitted changes, but no `crucible run`/`harden` ran recently. A passing test suite is not proof the app works or that the tests assert anything. Run `crucible run` (does it boot and drive?) and `crucible harden` (do the tests bite?), or `crucible check`, and address what they surface before reporting done. If verification genuinely does not apply to this change, say why and stop.";

const SESSION_CONTEXT: &str = "Crucible is active in this repo. Before reporting a change as tested or done, verify it: `crucible run` (does the app boot and drive?), `crucible harden` (do the tests actually constrain behavior?), `crucible check` (are the gates honest?). A green suite is not proof.";

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
// `stop_hook_active` are read from it; an unparsable payload falls back to the process
// cwd and a safe no-op.
pub fn run_hook(event: &str, input: &str) -> HookResult {
    let v: Value = serde_json::from_str(input).unwrap_or(Value::Null);
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
