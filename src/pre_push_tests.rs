use super::*;
use crate::config::Adapter;
use serde_json::json;
use std::fs;
use std::process::Command;

#[test]
fn hook_runs_crucible_check_accepts_active_invocations() {
    assert!(hook_runs_crucible_check("#!/bin/sh\ncrucible check || exit 1\n"));
    assert!(hook_runs_crucible_check("  /usr/local/bin/crucible check --repo .\n"));
    assert!(!hook_runs_crucible_check("# crucible check\n"));
    assert!(!hook_runs_crucible_check("// crucible check\n"));
    assert!(!hook_runs_crucible_check("echo crucible is great\n"));
    assert!(!hook_runs_crucible_check("crucible doctor\n"));
}

#[test]
fn inert_pre_push_wiring_does_not_count_as_running_check() {
    // Text present but exit status never affected — independence is fake.
    assert!(!hook_runs_crucible_check("crucible check || true\n"));
    assert!(!hook_runs_crucible_check("crucible check ||:\n"));
    assert!(!hook_runs_crucible_check("crucible check || exit 0\n"));
    assert!(!hook_runs_crucible_check("if false; then crucible check; fi\n"));
    assert!(!hook_runs_crucible_check("false && crucible check\n"));
    assert!(!hook_runs_crucible_check("echo crucible check\n"));
    assert!(!hook_runs_crucible_check("printf '%s\\n' crucible check\n"));
    // Still load-bearing when failure aborts the hook.
    assert!(hook_runs_crucible_check("crucible check || exit 1\n"));
    assert!(hook_runs_crucible_check("  /usr/local/bin/crucible check --repo .\n"));
    // Logging then invoking is still real.
    assert!(hook_runs_crucible_check("echo starting && crucible check || exit 1\n"));
}

#[test]
fn verify_pre_push_rejects_swallowed_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".githooks")).unwrap();
    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || true\n",
    )
    .unwrap();
    let adapter: Adapter = serde_json::from_value(json!({
        "gateRunner": { "file": "g", "checkerPattern": "(x)" },
        "prePush": ".githooks/pre-push"
    }))
    .unwrap();
    let f = verify_pre_push(root, &adapter);
    assert!(
        f.iter().any(|m| m.contains("does not run") || m.contains("inert")),
        "{f:?}"
    );
}

#[test]
fn verify_pre_push_requires_declared_hook() {
    let dir = tempfile::tempdir().unwrap();
    let adapter: Adapter = serde_json::from_value(json!({
        "gateRunner": { "file": "g", "checkerPattern": "(x)" }
    }))
    .unwrap();
    let f = verify_pre_push(dir.path(), &adapter);
    assert!(f.iter().any(|m| m.contains("prePush is required")), "{f:?}");
}

#[test]
fn verify_pre_push_requires_existing_file_that_runs_check() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".githooks")).unwrap();
    let adapter: Adapter = serde_json::from_value(json!({
        "gateRunner": { "file": "g", "checkerPattern": "(x)" },
        "prePush": ".githooks/pre-push"
    }))
    .unwrap();
    let missing = verify_pre_push(root, &adapter);
    assert!(
        missing.iter().any(|m| m.contains("does not exist")),
        "{missing:?}"
    );

    fs::write(root.join(".githooks/pre-push"), "#!/bin/sh\necho hi\n").unwrap();
    let no_check = verify_pre_push(root, &adapter);
    assert!(
        no_check.iter().any(|m| m.contains("does not run")),
        "{no_check:?}"
    );

    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || exit 1\n",
    )
    .unwrap();
    assert!(
        verify_pre_push(root, &adapter).is_empty(),
        "valid hook must pass"
    );
}

#[test]
fn same_commit_approval_with_judge_config_is_flagged() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // Minimal git repo with same-commit approvals + adapter.
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    // Identity required for commit on clean CI images.
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(root)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "test"])
        .current_dir(root)
        .status();

    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"gateRunner":{"file":"g","checkerPattern":"(x)"},"pinnedConfig":[".crucible/adapter.json"]}"#,
    )
    .unwrap();
    fs::write(root.join(".crucible/approvals.json"), "[]\n").unwrap();
    fs::write(root.join(".crucible/charter.json"), r#"{"gates":[]}"#).unwrap();

    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "self-approve"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );

    let adapter: Adapter = serde_json::from_value(json!({
        "gateRunner": { "file": "g", "checkerPattern": "(x)" },
        "approvals": ".crucible/approvals.json",
        "charter": ".crucible/charter.json",
        "pinnedConfig": [".crucible/adapter.json"],
    }))
    .unwrap();

    let f = audit_same_commit_approvals(root, &adapter);
    assert!(
        f.iter().any(|m| m.contains("together with judge config")),
        "HEAD co-commit must flag: {f:?}"
    );

    // A later unrelated commit makes the co-approval historical — not a live self-approve.
    fs::write(root.join("README"), "ok\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "README"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "move past"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        audit_same_commit_approvals(root, &adapter).is_empty(),
        "historical co-commit must not fail forever"
    );
}

#[test]
fn separate_approval_commit_is_clean() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(root)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "test"])
        .current_dir(root)
        .status();

    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"gateRunner":{"file":"g","checkerPattern":"(x)"},"pinnedConfig":[".crucible/adapter.json"]}"#,
    )
    .unwrap();
    fs::write(root.join(".crucible/charter.json"), r#"{"gates":[]}"#).unwrap();
    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "config only"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );

    fs::write(
        root.join(".crucible/approvals.json"),
        r#"[{"gate":"__config__","fingerprint":"x","approvedBy":"reviewer"}]"#,
    )
    .unwrap();
    assert!(
        Command::new("git")
            .args(["add", ".crucible/approvals.json"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "approve config"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );

    let adapter: Adapter = serde_json::from_value(json!({
        "gateRunner": { "file": "g", "checkerPattern": "(x)" },
        "approvals": ".crucible/approvals.json",
        "charter": ".crucible/charter.json",
        "pinnedConfig": [".crucible/adapter.json"],
    }))
    .unwrap();

    assert!(
        audit_same_commit_approvals(root, &adapter).is_empty(),
        "approval-only commit must pass"
    );
}

#[test]
fn shallow_clone_without_parent_does_not_false_positive() {
    // actions/checkout defaults to depth 1; git show HEAD then lists the whole
    // tree. The audit must not treat that as a live approvals+config co-commit.
    let src = tempfile::tempdir().unwrap();
    let root = src.path();
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    let _ = Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(root)
        .status();
    let _ = Command::new("git")
        .args(["config", "user.name", "test"])
        .current_dir(root)
        .status();

    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"gateRunner":{"file":"g","checkerPattern":"(x)"},"pinnedConfig":[".crucible/adapter.json"]}"#,
    )
    .unwrap();
    fs::write(root.join(".crucible/charter.json"), r#"{"gates":[]}"#).unwrap();
    fs::write(root.join(".crucible/approvals.json"), "[]\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "seed"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    fs::write(root.join("README"), "later\n").unwrap();
    assert!(
        Command::new("git")
            .args(["add", "README"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("git")
            .args(["commit", "-m", "unrelated tip"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );

    let shallow = tempfile::tempdir().unwrap();
    let shallow_path = shallow.path().join("clone");
    assert!(
        Command::new("git")
            .args([
                "clone",
                "--depth",
                "1",
                root.to_str().unwrap(),
                shallow_path.to_str().unwrap(),
            ])
            .status()
            .unwrap()
            .success()
    );

    let adapter: Adapter = serde_json::from_value(json!({
        "gateRunner": { "file": "g", "checkerPattern": "(x)" },
        "approvals": ".crucible/approvals.json",
        "charter": ".crucible/charter.json",
        "pinnedConfig": [".crucible/adapter.json"],
    }))
    .unwrap();

    assert!(
        audit_same_commit_approvals(&shallow_path, &adapter).is_empty(),
        "shallow tip without parent must not invent a co-commit"
    );
}
