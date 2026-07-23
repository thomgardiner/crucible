use super::*;

#[test]
fn should_block_only_when_adopted_dirty_stale_and_first_stop() {
    // The one case that nudges: adopted repo, first stop, enabled, dirty, stale.
    assert!(should_block_stop(true, false, false, true, false));
    // Every guard that must suppress it:
    assert!(
        !should_block_stop(false, false, false, true, false),
        "not adopted"
    );
    assert!(
        !should_block_stop(true, true, false, true, false),
        "already fired — never loop"
    );
    assert!(
        !should_block_stop(true, false, true, true, false),
        "disabled"
    );
    assert!(
        !should_block_stop(true, false, false, false, false),
        "no work to verify"
    );
    assert!(
        !should_block_stop(true, false, false, true, true),
        "recently verified"
    );
}

#[test]
fn receipt_roundtrips_and_reports_fresh() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!receipt_fresh(dir.path(), "run"), "no receipt yet");
    write_receipt(dir.path(), "run");
    assert!(receipt_fresh(dir.path(), "run"), "just written is fresh");
    assert!(verified_recently(dir.path()));
}

#[test]
fn a_flake_receipt_does_not_satisfy_the_verification_nudge() {
    // Determinism is not correctness: a flake success must never stand in for
    // run/harden (typed receipts).
    let dir = tempfile::tempdir().unwrap();
    write_receipt(dir.path(), "flake");
    assert!(
        receipt_fresh(dir.path(), "flake"),
        "the flake receipt itself is fine"
    );
    assert!(!verified_recently(dir.path()), "but it verifies no change");
}

#[test]
fn check_or_cover_alone_does_not_satisfy_the_stop_nudge() {
    // Agent lie: run only `crucible check` (or cover) and claim verified. Gate honesty
    // and reachability are not "tests bite" / "app runs".
    let dir = tempfile::tempdir().unwrap();
    write_receipt(dir.path(), "check");
    assert!(receipt_fresh(dir.path(), "check"));
    assert!(
        !verified_recently(dir.path()),
        "check-only must not clear the Stop nudge"
    );
    write_receipt(dir.path(), "cover");
    assert!(
        !verified_recently(dir.path()),
        "cover-only must not clear the Stop nudge"
    );
    write_receipt(dir.path(), "harden");
    assert!(
        verified_recently(dir.path()),
        "harden clears the nudge"
    );
}

#[test]
fn editing_the_worktree_after_a_receipt_invalidates_it() {
    // A receipt is bound to the tree it verified: new edits reopen the question.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "commit",
        "-qm",
        "init",
    ]);

    write_receipt(root, "run");
    assert!(verified_recently(root), "verified for this tree");

    std::fs::write(root.join("a.rs"), "fn a() { /* changed */ }\n").unwrap();
    assert!(
        !verified_recently(root),
        "an edit after verification must invalidate the receipt"
    );
}

#[test]
fn session_start_injects_context_only_in_adopted_repos() {
    let dir = tempfile::tempdir().unwrap();
    let payload = json!({ "cwd": dir.path() }).to_string();
    assert_eq!(
        run_hook("session-start", &payload).stdout,
        "",
        "silent in a non-adopted repo"
    );

    std::fs::create_dir_all(dir.path().join(".crucible")).unwrap();
    let out = run_hook("session-start", &payload);
    assert!(out.stdout.contains("Crucible is active"), "{}", out.stdout);
    assert!(out.stdout.contains("additionalContext"), "{}", out.stdout);
}

#[test]
fn stop_never_loops_when_already_active() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".crucible")).unwrap();
    let input = json!({ "cwd": dir.path(), "stop_hook_active": true }).to_string();
    assert_eq!(
        run_hook("stop", &input).stdout,
        "",
        "must not block a second time"
    );
}

#[test]
fn unknown_event_and_garbage_input_are_noops() {
    assert_eq!(run_hook("whatever", "{}").stdout, "");
    assert_eq!(run_hook("stop", "not json").stdout, "");
    // Empty JSON is parseable but has no cwd — uses process cwd. When that cwd is
    // this adopted repo with dirty work, Stop may block; that is intentional for a
    // real TUI payload. Garbage non-JSON must never block (safe no-op).
    assert_eq!(run_hook("stop", "{not json").stdout, "");
}

#[test]
fn editing_an_untracked_file_after_a_receipt_invalidates_it() {
    // `git diff HEAD` cannot see untracked contents; the fingerprint includes their
    // size+mtime so a rewrite reopens the question (Codex round 6).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(root.join("base.rs"), "fn b() {}\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "init",
    ]);
    std::fs::write(root.join("new.rs"), "fn n() {}\n").unwrap(); // untracked

    write_receipt(root, "run");
    assert!(verified_recently(root), "verified for this tree");

    std::fs::write(root.join("new.rs"), "fn n() { /* rewritten, longer */ }\n").unwrap();
    assert!(
        !verified_recently(root),
        "rewriting an untracked file must invalidate the receipt"
    );
}

#[test]
fn a_same_length_rewrite_of_an_untracked_file_invalidates_the_receipt() {
    // Content is hashed, not size+mtime, so an equal-length rewrite still invalidates —
    // and a file inside an untracked DIRECTORY counts too (Codex round 7).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(root.join("base.rs"), "fn b() {}\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "init",
    ]);
    std::fs::create_dir_all(root.join("newdir")).unwrap();
    std::fs::write(root.join("newdir/a.rs"), "fn n() { let v = 1; }\n").unwrap();

    write_receipt(root, "run");
    assert!(verified_recently(root));

    // Same byte length, different content, inside the untracked directory.
    std::fs::write(root.join("newdir/a.rs"), "fn n() { let v = 2; }\n").unwrap();
    assert!(
        !verified_recently(root),
        "a same-length rewrite in an untracked dir must invalidate the receipt"
    );
}

// ---- mutation-run kill-tests (self-audit) ----------------------------------

// Env-var reads are process-global; serialize the tests that touch or depend on
// CRUCIBLE_NO_NUDGE so they cannot race.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn an_expired_receipt_is_not_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let p = receipt_path(dir.path(), "run");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    // Timestamp just past the window, current (empty, non-git) tree fingerprint.
    let then = now_secs() - RECEIPT_MAX_AGE_SECS - 1;
    std::fs::write(
        &p,
        format!("CRUCIBLE-RECEIPT-v1\nrun\n{then}\n\n"),
    )
    .unwrap();
    assert!(
        !receipt_fresh(dir.path(), "run"),
        "expired must not be fresh"
    );
}

#[test]
fn forged_receipt_without_magic_is_not_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let p = receipt_path(dir.path(), "run");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    // Casual echo forgery: timestamp only, no magic/arm.
    std::fs::write(&p, format!("{}\n\n", now_secs())).unwrap();
    assert!(!receipt_fresh(dir.path(), "run"));
    // Wrong arm line must not satisfy a different arm's receipt path.
    std::fs::write(
        &p,
        format!("CRUCIBLE-RECEIPT-v1\nharden\n{}\n\n", now_secs()),
    )
    .unwrap();
    assert!(!receipt_fresh(dir.path(), "run"));
}

#[test]
fn nudge_disabled_reads_the_env_var() {
    let _guard = ENV_LOCK.lock().unwrap();
    assert!(!nudge_disabled(), "default: nudging enabled");
    unsafe { std::env::set_var("CRUCIBLE_NO_NUDGE", "1") };
    assert!(nudge_disabled());
    unsafe { std::env::remove_var("CRUCIBLE_NO_NUDGE") };
    assert!(!nudge_disabled());
}

#[test]
fn has_uncommitted_changes_reflects_the_worktree() {
    // Non-git and not adopted: no work to verify.
    let plain = tempfile::tempdir().unwrap();
    assert!(!has_uncommitted_changes(plain.path()));

    // Adopted without a usable git answer: fail closed (assume dirty).
    let adopted_plain = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(adopted_plain.path().join(".crucible")).unwrap();
    // Not a git repo — status fails; adoption still pressures verification.
    assert!(
        has_uncommitted_changes(adopted_plain.path()),
        "adopted + no git must not skip the nudge"
    );

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "init",
    ]);
    assert!(!has_uncommitted_changes(root), "clean tree");
    std::fs::write(root.join("a.rs"), "fn a() { /* dirty */ }\n").unwrap();
    assert!(has_uncommitted_changes(root), "dirty tree");
}

#[test]
fn a_dirty_unverified_adopted_repo_blocks_on_stop() {
    // The full stop path end to end: this is the hook's whole purpose, so the "stop"
    // dispatch arm must be exercised, not just its no-op branches.
    let _guard = ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::create_dir_all(root.join(".crucible")).unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "init",
    ]);
    std::fs::write(root.join("a.rs"), "fn a() { /* dirty */ }\n").unwrap();

    let payload = json!({ "cwd": root, "stop_hook_active": false }).to_string();
    let out = run_hook("stop", &payload);
    assert!(out.stdout.contains("\"block\""), "{}", out.stdout);

    // Verified via a change-verifying arm: the nudge clears.
    write_receipt(root, "run");
    assert_eq!(run_hook("stop", &payload).stdout, "");
}

#[test]
fn a_receipt_at_exactly_the_age_limit_is_still_fresh() {
    // The window is inclusive: stale strictly PAST the limit. Retry across a second
    // boundary so the age is measured deterministically.
    let dir = tempfile::tempdir().unwrap();
    for _ in 0..5 {
        let n1 = now_secs();
        let p = receipt_path(dir.path(), "run");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        // Empty fingerprint matches non-git tree_fingerprint.
        std::fs::write(
            &p,
            format!(
                "CRUCIBLE-RECEIPT-v1\nrun\n{}\n\n",
                n1 - RECEIPT_MAX_AGE_SECS
            ),
        )
        .unwrap();
        let fresh = receipt_fresh(dir.path(), "run");
        if now_secs() == n1 {
            assert!(fresh, "age == limit must still be fresh");
            return;
        }
    }
    panic!("clock never stable across five attempts");
}

#[test]
fn edit_past_the_leading_sample_still_invalidates_the_receipt() {
    // Full-file content digest: a change after the first 4KB must still reopen the
    // verification question (not only size/mtime/leading bytes).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    let mut body = vec![b'a'; 5000];
    body.push(b'\n');
    std::fs::write(root.join("big.rs"), &body).unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "init",
    ]);

    write_receipt(root, "run");
    assert!(verified_recently(root));

    body[4500] = b'Z'; // past the old 4KB sample window
    std::fs::write(root.join("big.rs"), &body).unwrap();
    assert!(
        !verified_recently(root),
        "edit past 4KB must invalidate the full-content fingerprint"
    );
}

#[test]
fn staging_an_edit_after_a_receipt_invalidates_it() {
    // `git ls-files -m` misses a file whose worktree matches the index after `git add`.
    // The fingerprint also folds in staged (index-vs-HEAD) changes (Codex resource review).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    git(&["add", "-A"]);
    git(&[
        "-c",
        "user.email=t@t",
        "-c",
        "user.name=t",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "init",
    ]);

    write_receipt(root, "run");
    assert!(verified_recently(root), "verified for this tree");

    // Edit then stage: worktree == index, so ls-files -m alone would miss it.
    std::fs::write(root.join("a.rs"), "fn a() { /* changed and staged */ }\n").unwrap();
    git(&["add", "a.rs"]);
    assert!(
        !verified_recently(root),
        "a staged edit after verification must invalidate the receipt"
    );
}
