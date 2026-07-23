use super::*;
use crate::charter::check_charter;
use crate::config::{Adapter, Approval, Ledger, load_json};
use serde_json::json;
use std::fs;

// A repo whose gate is declared but not yet approved, laid down on disk (approve reads
// and rewrites the real charter/approvals files).
fn repo(pinned: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("checks")).unwrap();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::create_dir_all(root.join(".githooks")).unwrap();
    fs::write(root.join("checks/check-foo.mjs"), "process.exit(0)\n").unwrap();
    fs::write(root.join("gate-runner.txt"), "node checks/check-foo.mjs\n").unwrap();
    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || exit 1\n",
    )
    .unwrap();
    let adapter = json!({
        "repo": "fixture",
        "charter": ".crucible/charter.json",
        "approvals": ".crucible/approvals.json",
        "gateRunner": { "file": "gate-runner.txt", "checkerPattern": "node (checks/check-[a-z-]+\\.mjs)" },
        "highRiskUnits": [],
        "prePush": ".githooks/pre-push",
        "pinnedConfig": if pinned { json!([".crucible/adapter.json"]) } else { json!([]) },
    });
    fs::write(
        root.join(".crucible/adapter.json"),
        serde_json::to_string_pretty(&adapter).unwrap(),
    )
    .unwrap();
    let charter = json!({
        "_note": "keep me across the rewrite",
        "gates": [{ "id": "foo", "rule": "foo must hold", "tier": "T1", "checker": "checks/check-foo.mjs", "blockingCondition": "always" }],
    });
    fs::write(
        root.join(".crucible/charter.json"),
        serde_json::to_string_pretty(&charter).unwrap(),
    )
    .unwrap();
    dir
}

fn load(root: &std::path::Path) -> (Adapter, Ledger, Vec<Approval>) {
    let adapter: Adapter = load_json(&root.join(".crucible/adapter.json")).unwrap();
    let ledger: Ledger = load_json(&root.join(".crucible/charter.json")).unwrap();
    let approvals: Vec<Approval> =
        load_json(&root.join(".crucible/approvals.json")).unwrap_or_default();
    (adapter, ledger, approvals)
}

#[test]
fn approving_the_gate_and_config_makes_check_pass() {
    let dir = repo(true);
    let root = dir.path();
    let adapter: Adapter = load_json(&root.join(".crucible/adapter.json")).unwrap();

    // Before approval: not backed → fails.
    let (a, l, ap) = load(root);
    assert!(!check_charter(root, &l, &a, &ap).failures.is_empty());

    approve(root, &adapter, "foo", "reviewer", "").unwrap();
    approve(root, &adapter, "__config__", "reviewer", "").unwrap();

    let (a, l, ap) = load(root);
    let r = check_charter(root, &l, &a, &ap);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
}

#[test]
fn an_empty_pinned_config_still_requires_a_config_approval() {
    // The trust set is derived, so even a repo that pins nothing must have its adapter and
    // charter approved — approving only the gate is not enough (Codex rounds 2–3 #6).
    let dir = repo(false);
    let root = dir.path();
    let adapter: Adapter = load_json(&root.join(".crucible/adapter.json")).unwrap();
    approve(root, &adapter, "foo", "reviewer", "").unwrap();
    let (a, l, ap) = load(root);
    let r = check_charter(root, &l, &a, &ap);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("judge config") && f.contains("not backed")),
        "{:?}",
        r.failures
    );

    // Approving __config__ over the derived set clears it.
    approve(root, &adapter, "__config__", "reviewer", "").unwrap();
    let (a, l, ap) = load(root);
    let r = check_charter(root, &l, &a, &ap);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
}

#[test]
fn approve_preserves_hand_written_charter_keys() {
    let dir = repo(false);
    let root = dir.path();
    let adapter: Adapter = load_json(&root.join(".crucible/adapter.json")).unwrap();
    approve(root, &adapter, "foo", "reviewer", "").unwrap();
    let text = fs::read_to_string(root.join(".crucible/charter.json")).unwrap();
    assert!(
        text.contains("keep me across the rewrite"),
        "the _note must survive the rewrite"
    );
    assert!(
        text.contains("oracleSha256"),
        "approve pins the oracle digest"
    );
}

#[test]
fn approving_config_pins_the_judge_configuration() {
    let dir = repo(true);
    let root = dir.path();
    let adapter: Adapter = load_json(&root.join(".crucible/adapter.json")).unwrap();
    approve(root, &adapter, "foo", "reviewer", "").unwrap();
    approve(root, &adapter, "__config__", "reviewer", "").unwrap();
    let (a, l, ap) = load(root);
    let r = check_charter(root, &l, &a, &ap);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
}

#[test]
fn approve_refuses_to_pin_a_judge_file_that_does_not_parse() {
    // Byte-level approval of a broken recipe would pass check while its arm cannot run.
    let dir = repo(true);
    let root = dir.path();
    let adapter: Adapter = load_json(&root.join(".crucible/adapter.json")).unwrap();
    std::fs::write(root.join(".crucible/mutation.json"), "{not json").unwrap();
    let err = approve(root, &adapter, "__config__", "reviewer", "").unwrap_err();
    assert!(format!("{err:#}").contains("does not parse"), "{err:#}");
}
