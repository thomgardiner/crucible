//! Pre-push wiring and approval-trail audits.
//!
//! Threat model is a single developer + agents that commit as that developer.
//! Cryptographic "approver ≠ author" is therefore impossible in-core. Independence
//! is the combination of:
//! 1. A pre-push hook that exists and actually runs the gates (verified here).
//! 2. An auditable approval trail: a `__config__` approval must not land in the
//!    same git commit as the config it blesses (flagged here when git is available).
//!
//! Stronger claims would be a lie in the single-dev model — see POSITIONING.md.

use crate::charter::judge_config_paths;
use crate::config::Adapter;
use std::path::{Path, PathBuf};
use std::process::Command;

fn resolve(repo_root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(p)
    }
}

/// Line is disabled for wiring purposes if `#` or `//` appears before the match.
fn is_commented_line(line: &str, match_at: usize) -> bool {
    let before = &line[..match_at.min(line.len())];
    before.contains('#') || before.contains("//")
}

/// Same-line runtime neutering: the invocation is present as text but never
/// affects exit status. Shared by pre-push and gate-runner wiring.
pub fn line_is_neutered(line: &str, match_at: usize) -> bool {
    let lower = line.to_ascii_lowercase();
    let at = match_at.min(lower.len());
    let before = &lower[..at];
    let after = &lower[at..];
    // Disabled by false condition before the call.
    if before.contains("if false") || before.contains("false &&") || before.contains("false;")
    {
        return true;
    }
    // Exit status swallowed after the call.
    if after.contains("|| true")
        || after.contains("||:")
        || after.contains("|| :")
        || after.contains("|| exit 0")
        || after.contains("||exit 0")
    {
        return true;
    }
    false
}

/// True when the match at `match_index` in multi-line `text` sits on a neutered line.
pub fn match_is_neutered(text: &str, match_index: usize) -> bool {
    let line_start = text[..match_index].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = text[match_index..]
        .find('\n')
        .map(|i| match_index + i)
        .unwrap_or(text.len());
    let line = &text[line_start..line_end];
    line_is_neutered(line, match_index - line_start)
}

/// True if the hook body has an active (non-commented, non-neutered) invocation of
/// `crucible check`.
pub fn hook_runs_crucible_check(text: &str) -> bool {
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        // Allow path-qualified binaries and common wrappers: crucible check, ./crucible check
        if let Some(idx) = lower.find("crucible") {
            let rest = &lower[idx + "crucible".len()..];
            let rest = rest.trim_start();
            if rest.starts_with("check")
                && !is_commented_line(line, idx)
                && !line_is_neutered(line, idx)
            {
                return true;
            }
        }
    }
    false
}

/// Failures when the adapter's pre-push claim is missing, the file is gone, or the
/// hook does not actually run `crucible check`.
pub fn verify_pre_push(repo_root: &Path, adapter: &Adapter) -> Vec<String> {
    let mut failures = vec![];
    let Some(rel) = adapter.pre_push.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        failures.push(
            "adapter.prePush is required — independence is enforced at pre-push, so the \
             adapter must name a hook file that runs `crucible check` (see POSITIONING.md)"
                .into(),
        );
        return failures;
    };

    let path = resolve(repo_root, rel);
    if !path.exists() {
        failures.push(format!(
            "adapter.prePush \"{rel}\" does not exist — wire a hook that runs `crucible check` \
             (init scaffolds .githooks/pre-push) so independence is a verified fact, not a claim"
        ));
        return failures;
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            failures.push(format!("reading adapter.prePush \"{rel}\": {e}"));
            return failures;
        }
    };

    if !hook_runs_crucible_check(&text) {
        failures.push(format!(
            "adapter.prePush \"{rel}\" does not run `crucible check` on an active line — \
             a hook that never invokes the honesty gate cannot enforce independence \
             (commented out, `if false`, or `|| true` / `|| exit 0` counts as inert)"
        ));
    }

    failures
}

/// Soft adoption signal for `doctor`: is core.hooksPath aimed at the pre-push file's dir?
pub fn hooks_path_status(repo_root: &Path, adapter: &Adapter) -> Option<String> {
    let rel = adapter.pre_push.as_deref()?.trim();
    if rel.is_empty() {
        return None;
    }
    let path = resolve(repo_root, rel);
    let Some(hooks_path) = git_stdout(repo_root, &["config", "--get", "core.hooksPath"]) else {
        return Some(format!(
            "git core.hooksPath is unset — run `git config core.hooksPath {}` so \"{rel}\" fires on push",
            path.parent()
                .map(|p| p
                    .strip_prefix(repo_root)
                    .unwrap_or(p)
                    .display()
                    .to_string())
                .unwrap_or_else(|| ".githooks".into())
        ));
    };
    let hooks_path = hooks_path.trim().replace('\\', "/");
    let expected = path
        .parent()
        .map(|p| {
            p.strip_prefix(repo_root)
                .unwrap_or(p)
                .display()
                .to_string()
                .replace('\\', "/")
        })
        .unwrap_or_default();
    if hooks_path == expected
        || hooks_path.ends_with(expected.trim_start_matches("./"))
        || rel.starts_with(hooks_path.trim_start_matches("./"))
    {
        None
    } else {
        Some(format!(
            "git core.hooksPath is \"{hooks_path}\" but adapter.prePush is \"{rel}\" — \
             point hooksPath at the hook directory so independence fires on push"
        ))
    }
}

fn git_ok(repo_root: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(repo_root)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn git_stdout(repo_root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Map a path under `repo_root` to a pathspec relative to the git worktree root.
fn git_pathspec(repo_root: &Path, rel: &str) -> Option<String> {
    let toplevel = git_stdout(repo_root, &["rev-parse", "--show-toplevel"])?
        .trim()
        .replace('\\', "/");
    let abs = resolve(repo_root, rel)
        .canonicalize()
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let top = Path::new(&toplevel)
        .canonicalize()
        .ok()?
        .to_string_lossy()
        .replace('\\', "/");
    let stripped = abs.strip_prefix(&top)?.trim_start_matches('/');
    Some(stripped.to_string())
}

/// Flag when the latest commit that touches the approvals log also changes any
/// judge-config path. That is a self-approval of a weakening, not an independent
/// audit trail. Skip when not a git checkout or git is unavailable.
pub fn audit_same_commit_approvals(repo_root: &Path, adapter: &Adapter) -> Vec<String> {
    let mut failures = vec![];
    if !git_ok(repo_root) {
        return failures;
    }

    let approvals_rel = adapter.approvals.as_str();
    let approvals_path = resolve(repo_root, approvals_rel);
    if !approvals_path.exists() {
        return failures;
    }

    // Pathspecs must be relative to the git worktree root (repo_root may be a subdir).
    let Some(approvals_spec) = git_pathspec(repo_root, approvals_rel) else {
        return failures;
    };

    // Last commit that touched the approvals file (empty if never committed).
    let Some(commit) = git_stdout(
        repo_root,
        &["log", "-1", "--format=%H", "--", &approvals_spec],
    )
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty()) else {
        return failures;
    };

    // Only the tip matters for the agent threat: a historical monorepo dump that once
    // co-committed approvals with config is not a live self-approval. Flag when the
    // *current* HEAD commit is the one that last wrote approvals alongside judge config.
    let Some(head) = git_stdout(repo_root, &["rev-parse", "HEAD"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    else {
        return failures;
    };
    if commit != head {
        return failures;
    }

    let Some(names) = git_stdout(repo_root, &["show", "--name-only", "--pretty=format:", &commit])
    else {
        return failures;
    };

    let changed: std::collections::HashSet<String> = names
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.replace('\\', "/"))
        .collect();

    if !changed.contains(&approvals_spec) {
        return failures;
    }

    let mut co_changed: Vec<String> = Vec::new();
    for p in judge_config_paths(repo_root, adapter) {
        let Some(spec) = git_pathspec(repo_root, &p) else {
            continue;
        };
        if changed.contains(&spec) {
            co_changed.push(p);
        }
    }
    co_changed.sort();
    co_changed.dedup();

    if !co_changed.is_empty() {
        failures.push(format!(
            "approvals log was last committed together with judge config ({}) in commit {} — \
             a __config__ (or gate) approval must be a separate commit from the change it \
             blesses so the audit trail shows an explicit 'I approved this' step an agent \
             cannot hide inside a larger diff",
            co_changed.join(", "),
            &commit[..commit.len().min(12)]
        ));
    }

    failures
}

#[cfg(test)]
#[path = "pre_push_tests.rs"]
mod tests;
