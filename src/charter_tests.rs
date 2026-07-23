use super::*;
use crate::config::{Adapter, Approval, Ledger};
use serde_json::json;
use std::fs;
use tempfile::TempDir;

// A fixture repo plus an approval for its one gate, so the honest baseline reflects
// the approval-backed model (the pin alone is not trusted).
fn make_repo() -> (TempDir, Adapter, Ledger, Vec<Approval>) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("checks")).unwrap();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(root.join("checks/check-foo.mjs"), "process.exit(0)\n").unwrap();
    fs::write(
        root.join("gate-runner.txt"),
        "step A\nnode checks/check-foo.mjs\nstep B\n",
    )
    .unwrap();
    // Pre-push is load-bearing: must exist and run `crucible check`.
    fs::create_dir_all(root.join(".githooks")).unwrap();
    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || exit 1\n",
    )
    .unwrap();
    // The adapter is the trust root and must pin itself, so it is written to its canonical
    // path and lists that path in pinnedConfig — the honest baseline the real model requires.
    let adapter_json = json!({
        "repo": "fixture",
        "gateRunner": { "file": "gate-runner.txt", "checkerPattern": "node (checks/check-[a-z-]+\\.mjs)" },
        "highRiskUnits": ["payments"],
        "prePush": ".githooks/pre-push",
        "pinnedConfig": [".crucible/adapter.json"],
    });
    fs::write(
        root.join(".crucible/adapter.json"),
        serde_json::to_string_pretty(&adapter_json).unwrap(),
    )
    .unwrap();
    let adapter: Adapter = serde_json::from_value(adapter_json).unwrap();
    let ledger: Ledger = serde_json::from_value(json!({
        "gates": [{
            "id": "foo", "rule": "foo must hold", "tier": "T1",
            "checker": "checks/check-foo.mjs",
            "oracleSha256": sha256_hex_of_file(&root.join("checks/check-foo.mjs")).unwrap(),
            "blockingCondition": "always",
        }],
    }))
    .unwrap();
    let fp = gate_fingerprint(root, &ledger.gates[0]).unwrap();
    let cfg_fp = config_fingerprint(root, &judge_config_paths(root, &adapter)).unwrap();
    let approvals: Vec<Approval> = serde_json::from_value(json!([
        { "gate": "foo", "fingerprint": fp, "approvedBy": "seed" },
        { "gate": "__config__", "fingerprint": cfg_fp, "approvedBy": "seed" },
    ]))
    .unwrap();
    (dir, adapter, ledger, approvals)
}

fn pin_of(root: &std::path::Path, checker: &str) -> String {
    sha256_hex_of_file(&root.join(checker)).unwrap()
}

#[test]
fn approval_backed_charter_passes() {
    let (dir, adapter, ledger, approvals) = make_repo();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
}

#[test]
fn editing_the_checker_without_a_fresh_approval_fails() {
    let (dir, adapter, ledger, approvals) = make_repo();
    fs::write(
        dir.path().join("checks/check-foo.mjs"),
        "process.exit(0) // now a no-op\n",
    )
    .unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert_eq!(r.failures.len(), 1, "{:?}", r.failures);
    assert!(r.failures[0].contains("not backed by an independent approval"));
}

#[test]
fn editing_checker_and_pin_together_still_fails() {
    let (dir, adapter, mut ledger, approvals) = make_repo();
    // Weaken the checker and update its own ledger pin — the exact bypass a pin-only
    // comparison would wave through.
    fs::write(
        dir.path().join("checks/check-foo.mjs"),
        "process.exit(0) // weakened\n",
    )
    .unwrap();
    ledger.gates[0].oracle_sha256 = Some(pin_of(dir.path(), "checks/check-foo.mjs"));
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert_eq!(r.failures.len(), 1, "{:?}", r.failures);
    assert!(r.failures[0].contains("not backed by an independent approval"));
}

#[test]
fn a_fresh_approval_matching_new_bytes_clears_the_failure() {
    let (dir, adapter, mut ledger, _) = make_repo();
    fs::write(
        dir.path().join("checks/check-foo.mjs"),
        "process.exit(0) // improved\n",
    )
    .unwrap();
    ledger.gates[0].oracle_sha256 = Some(pin_of(dir.path(), "checks/check-foo.mjs"));
    let fp = gate_fingerprint(dir.path(), &ledger.gates[0]).unwrap();
    let cfg_fp = config_fingerprint(dir.path(), &judge_config_paths(dir.path(), &adapter)).unwrap();
    let approvals: Vec<Approval> = serde_json::from_value(json!([
        { "gate": "foo", "fingerprint": fp, "approvedBy": "reviewer" },
        { "gate": "__config__", "fingerprint": cfg_fp, "approvedBy": "reviewer" },
    ]))
    .unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(r.failures.is_empty(), "{:?}", r.failures);
}

#[test]
fn a_commented_out_invocation_does_not_count_as_wired() {
    let (dir, adapter, ledger, approvals) = make_repo();
    fs::write(
        dir.path().join("gate-runner.txt"),
        "step A\n# node checks/check-foo.mjs\nstep B\n",
    )
    .unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("declared tier T1 but its checker") && f.contains("is not wired")),
        "{:?}",
        r.failures
    );
}

#[test]
fn a_block_commented_invocation_does_not_count_as_wired() {
    // Codex round 3: `/* node checks/check-foo.mjs */` never executes, so it must not
    // satisfy the T1 wiring check.
    let (dir, adapter, ledger, approvals) = make_repo();
    fs::write(
        dir.path().join("gate-runner.txt"),
        "/*\nnode checks/check-foo.mjs\n*/\nstep B\n",
    )
    .unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("declared tier T1 but its checker") && f.contains("is not wired")),
        "{:?}",
        r.failures
    );

    // A closed block before the invocation does not swallow it — still wired.
    fs::write(
        dir.path().join("gate-runner.txt"),
        "/* preamble */\nnode checks/check-foo.mjs\n",
    )
    .unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        !r.failures.iter().any(|f| f.contains("is not wired")),
        "{:?}",
        r.failures
    );
}

#[test]
fn a_wired_checker_missing_from_the_charter_fails() {
    let (dir, adapter, ledger, approvals) = make_repo();
    fs::write(dir.path().join("checks/check-bar.mjs"), "process.exit(0)\n").unwrap();
    fs::write(
        dir.path().join("gate-runner.txt"),
        "node checks/check-foo.mjs\nnode checks/check-bar.mjs\n",
    )
    .unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert_eq!(r.failures.len(), 1, "{:?}", r.failures);
    assert!(
        r.failures[0].contains("check-bar.mjs")
            && r.failures[0].contains("not registered in the charter")
    );
}

#[test]
fn a_t1_gate_not_wired_into_the_required_lane_fails() {
    let (dir, adapter, ledger, approvals) = make_repo();
    fs::write(dir.path().join("gate-runner.txt"), "step A\nstep B\n").unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("declared tier T1 but its checker") && f.contains("is not wired"))
    );
}

#[test]
fn a_missing_checker_file_fails() {
    let (dir, adapter, mut ledger, approvals) = make_repo();
    ledger.gates[0].checker = Some("checks/check-missing.mjs".into());
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("checker file does not exist"))
    );
}

#[test]
fn gate_fingerprint_is_not_confusable_by_delimiters() {
    // checker "c|x" + rule "r" and checker "c" + rule "x|r" concatenate identically under
    // an unescaped `|`; length-prefixing keeps them distinct (Codex P1 #9).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("c|x"), "x\n").unwrap();
    fs::write(root.join("c"), "x\n").unwrap();
    let g1: crate::config::Gate = serde_json::from_value(
        json!({ "id": "a", "rule": "r", "tier": "T1", "checker": "c|x", "blockingCondition": "always" }),
    )
    .unwrap();
    let g2: crate::config::Gate = serde_json::from_value(
        json!({ "id": "a", "rule": "x|r", "tier": "T1", "checker": "c", "blockingCondition": "always" }),
    )
    .unwrap();
    assert_ne!(
        gate_fingerprint(root, &g1).unwrap(),
        gate_fingerprint(root, &g2).unwrap(),
        "distinct gates must not share a fingerprint"
    );
}

#[test]
fn a_trustedfile_change_without_a_fresh_approval_fails() {
    let (dir, adapter, mut ledger, _) = make_repo();
    fs::write(dir.path().join("fixture.txt"), "original\n").unwrap();
    ledger.gates[0].trusted_files = serde_json::from_value(json!([
        { "path": "fixture.txt", "sha256": pin_of(dir.path(), "fixture.txt") }
    ]))
    .unwrap();
    let fp = gate_fingerprint(dir.path(), &ledger.gates[0]).unwrap();
    let approvals: Vec<Approval> = serde_json::from_value(
        json!([{ "gate": "foo", "fingerprint": fp, "approvedBy": "reviewer" }]),
    )
    .unwrap();
    fs::write(dir.path().join("fixture.txt"), "tampered\n").unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("not backed by an independent approval"))
    );
}

#[test]
fn pinned_judge_config_must_be_independently_approved() {
    let (dir, mut adapter, ledger, _) = make_repo();
    let root = dir.path();
    fs::write(
        root.join("cfg.json"),
        "{\"highRiskUnits\":[\"payments\"]}\n",
    )
    .unwrap();
    // Pin a recipe alongside the mandatory adapter self-pin.
    adapter.pinned_config = vec![".crucible/adapter.json".into(), "cfg.json".into()];
    let gate_fp = gate_fingerprint(root, &ledger.gates[0]).unwrap();
    let gate_approval: Vec<Approval> = serde_json::from_value(
        json!([{ "gate": "foo", "fingerprint": gate_fp, "approvedBy": "seed" }]),
    )
    .unwrap();

    let unpinned = check_charter(root, &ledger, &adapter, &gate_approval);
    assert!(
        unpinned
            .failures
            .iter()
            .any(|f| f.contains("judge config")
                && f.contains("not backed by an independent approval"))
    );

    let fp = config_fingerprint(root, &judge_config_paths(root, &adapter)).unwrap();
    let mut with_cfg = gate_approval.clone();
    with_cfg.extend::<Vec<Approval>>(
        serde_json::from_value(
            json!([{ "gate": "__config__", "fingerprint": fp, "approvedBy": "reviewer" }]),
        )
        .unwrap(),
    );
    let pinned = check_charter(root, &ledger, &adapter, &with_cfg);
    assert!(pinned.failures.is_empty(), "{:?}", pinned.failures);

    // weakening the config breaks the pin
    fs::write(root.join("cfg.json"), "{\"highRiskUnits\":[]}\n").unwrap();
    let weakened = check_charter(root, &ledger, &adapter, &with_cfg);
    assert!(
        weakened
            .failures
            .iter()
            .any(|f| f.contains("judge config") && f.contains("not backed"))
    );
}

#[test]
fn a_whitespace_only_reason_does_not_satisfy_a_t3_gate() {
    // Codex round 3: reason was checked with is_none(), so "   " passed. A prose gate's
    // rationale must be real, matching the non-blank rule already applied to approvedBy.
    let (dir, adapter, mut ledger, approvals) = make_repo();
    ledger.gates[0].tier = "T3".into();
    ledger.gates[0].reason = Some("   ".into());
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("T3") && f.contains("non-empty \"reason\"")),
        "{:?}",
        r.failures
    );
}

#[test]
fn the_trust_set_is_derived_and_cannot_be_opted_out_of() {
    // Codex round 3: the trust set is derived, so the adapter and its judges are always
    // fingerprinted even when pinnedConfig is empty — there is no opt-out.
    let (dir, adapter, ledger, approvals) = make_repo();
    let root = dir.path();
    let paths = judge_config_paths(root, &adapter);
    assert!(paths.iter().any(|p| p == ".crucible/adapter.json"));
    assert!(paths.iter().any(|p| p == ".crucible/charter.json"));

    // Weakening the adapter file itself invalidates the seeded __config__ approval.
    fs::write(
        root.join(".crucible/adapter.json"),
        "{\"highRiskUnits\":[]}\n",
    )
    .unwrap();
    let r = check_charter(root, &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("judge config") && f.contains("not backed")),
        "{:?}",
        r.failures
    );
}

#[test]
fn adding_a_waivers_file_invalidates_the_config_approval() {
    // Codex round 3: broadening .crucible/mutation-waivers.json must never survive an old
    // __config__ approval — the file joins the derived set the moment it exists.
    let (dir, adapter, ledger, approvals) = make_repo();
    let root = dir.path();
    let honest = check_charter(root, &ledger, &adapter, &approvals);
    assert!(honest.failures.is_empty(), "{:?}", honest.failures);

    fs::write(
        root.join(".crucible/mutation-waivers.json"),
        "[{\"file\":\"src/pay.rs\",\"line\":1,\"reason\":\"broad\"}]\n",
    )
    .unwrap();
    let r = check_charter(root, &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("judge config") && f.contains("not backed")),
        "{:?}",
        r.failures
    );
}

#[test]
fn adding_assertion_helpers_invalidates_the_config_approval() {
    // Codex round 4: assertionHelpers changes what test-smells accepts as an assertion, so
    // .crucible/test-smells.json is a judge and joins the derived set when present.
    let (dir, adapter, ledger, approvals) = make_repo();
    let root = dir.path();
    fs::write(
        root.join(".crucible/test-smells.json"),
        "{\"assertionHelpers\":[\"pretend_assert\"]}\n",
    )
    .unwrap();
    let r = check_charter(root, &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("judge config") && f.contains("not backed")),
        "{:?}",
        r.failures
    );
}

#[test]
fn an_approval_with_no_approved_by_does_not_count() {
    let (dir, adapter, ledger, _) = make_repo();
    let fp = gate_fingerprint(dir.path(), &ledger.gates[0]).unwrap();
    let approvals: Vec<Approval> =
        serde_json::from_value(json!([{ "gate": "foo", "fingerprint": fp }])).unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("not backed by an independent approval"))
    );
}

#[test]
fn validate_ledger_rejects_a_t3_rule_with_no_reason() {
    let ledger: Ledger =
        serde_json::from_value(json!({ "gates": [{ "id": "x", "rule": "r", "tier": "T3" }] }))
            .unwrap();
    let failures = validate_ledger(&ledger);
    assert!(
        failures
            .iter()
            .any(|f| f.contains("T3") && f.contains("requires a non-empty \"reason\""))
    );
}

#[test]
fn validate_ledger_rejects_unknown_tier_and_duplicate_id() {
    let ledger: Ledger = serde_json::from_value(json!({
        "gates": [
            { "id": "dup", "rule": "r", "tier": "T1", "checker": "c", "oracleSha256": "h" },
            { "id": "dup", "rule": "r", "tier": "T9", "checker": "c", "oracleSha256": "h" },
        ],
    }))
    .unwrap();
    let failures = validate_ledger(&ledger);
    assert!(failures.iter().any(|f| f.contains("duplicate id")));
    assert!(failures.iter().any(|f| f.contains("tier must be one of")));
}

#[test]
fn audit_reports_counts_prose_and_undeclared() {
    let (dir, adapter, mut ledger, _) = make_repo();
    ledger.gates.push(
        serde_json::from_value(json!({
            "id": "manual-rule", "rule": "humans review money paths", "tier": "T3",
            "reason": "judgement call, no mechanical check yet",
        }))
        .unwrap(),
    );
    fs::write(dir.path().join("checks/check-baz.mjs"), "process.exit(0)\n").unwrap();
    fs::write(
        dir.path().join("gate-runner.txt"),
        "node checks/check-foo.mjs\nnode checks/check-baz.mjs\n",
    )
    .unwrap();
    let rep = audit_charter(dir.path(), &ledger, &adapter);
    let count = |t: &str| {
        rep.counts
            .iter()
            .find(|(k, _)| k == t)
            .map(|(_, n)| *n)
            .unwrap()
    };
    assert_eq!(count("T1"), 1);
    assert_eq!(count("T3"), 1);
    assert_eq!(rep.prose_only[0].id, "manual-rule");
    assert_eq!(rep.undeclared, vec!["checks/check-baz.mjs".to_string()]);
}

// ---- mutation-run kill-tests (self-audit) ----------------------------------

#[test]
fn validate_adapter_requires_runner_file_and_pattern() {
    let empty: Adapter = serde_json::from_value(json!({})).unwrap();
    let failures = validate_adapter(&empty);
    assert!(
        failures.iter().any(|f| f.contains("gateRunner.file")),
        "{failures:?}"
    );
    assert!(
        failures.iter().any(|f| f.contains("checkerPattern")),
        "{failures:?}"
    );
}

#[test]
fn validate_ledger_tier_rules_hold_exactly() {
    // A T1 gate without checker/oracleSha256 fails …
    let l: Ledger = serde_json::from_value(json!({
        "gates": [{ "id": "a", "rule": "r", "tier": "T1" }],
    }))
    .unwrap();
    let f = validate_ledger(&l);
    assert!(
        f.iter().any(|x| x.contains("requires a \"checker\"")),
        "{f:?}"
    );
    assert!(f.iter().any(|x| x.contains("oracleSha256")), "{f:?}");

    // … a T2 gate is a gate too …
    let l: Ledger = serde_json::from_value(json!({
        "gates": [{ "id": "a", "rule": "r", "tier": "T2" }],
    }))
    .unwrap();
    assert!(
        validate_ledger(&l)
            .iter()
            .any(|x| x.contains("requires a \"checker\""))
    );

    // … while a T3 prose gate with a real reason is fully valid, and an advisory
    // gate with a reason is too.
    let l: Ledger = serde_json::from_value(json!({
        "gates": [
            { "id": "p", "rule": "r", "tier": "T3", "reason": "documented tradeoff" },
        ],
    }))
    .unwrap();
    assert_eq!(validate_ledger(&l), Vec::<String>::new());

    // A trustedFiles entry with a path but no sha256 fails.
    let l: Ledger = serde_json::from_value(json!({
        "gates": [{
            "id": "a", "rule": "r", "tier": "T3", "reason": "why",
            "trustedFiles": [{ "path": "fixture.txt" }],
        }],
    }))
    .unwrap();
    assert!(
        validate_ledger(&l)
            .iter()
            .any(|x| x.contains("needs both \"path\" and \"sha256\""))
    );
}

#[test]
fn a_hash_on_the_previous_line_does_not_comment_the_invocation() {
    // The line-start offset must land AFTER the newline: a `#` ending the previous
    // line is not a comment marker for this line.
    let (dir, adapter, ledger, approvals) = make_repo();
    fs::write(
        dir.path().join("gate-runner.txt"),
        "step A#\nnode checks/check-foo.mjs\n",
    )
    .unwrap();
    let r = check_charter(dir.path(), &ledger, &adapter, &approvals);
    assert!(
        !r.failures.iter().any(|f| f.contains("is not wired")),
        "{:?}",
        r.failures
    );
}

#[test]
fn only_genuinely_missing_trusted_files_are_reported_missing() {
    let (dir, adapter, mut ledger, _) = make_repo();
    let root = dir.path();
    fs::write(root.join("present.txt"), "here\n").unwrap();
    ledger.gates[0].trusted_files = serde_json::from_value(json!([
        { "path": "present.txt", "sha256": sha256_hex_of_file(&root.join("present.txt")).unwrap() },
        { "path": "absent.txt", "sha256": "0000" },
    ]))
    .unwrap();
    let r = check_charter(root, &ledger, &adapter, &[]);
    assert!(
        r.failures
            .iter()
            .any(|f| f.contains("trustedFile missing: absent.txt")),
        "{:?}",
        r.failures
    );
    assert!(
        !r.failures
            .iter()
            .any(|f| f.contains("trustedFile missing: present.txt")),
        "{:?}",
        r.failures
    );
}

#[test]
fn a_stale_oracle_pin_warns_and_a_fresh_one_does_not() {
    // Approval-backed but the ledger's oracleSha256 lags the approved checker: warn.
    let (dir, adapter, mut ledger, approvals) = make_repo();
    ledger.gates[0].oracle_sha256 = Some("stale-digest".into());
    // Refresh the gate approval for the (unchanged) checker but stale ledger pin.
    let fp = gate_fingerprint(dir.path(), &ledger.gates[0]).unwrap();
    let mut ap = approvals.clone();
    ap.push(
        serde_json::from_value(json!({ "gate": "foo", "fingerprint": fp, "approvedBy": "seed" }))
            .unwrap(),
    );
    let r = check_charter(dir.path(), &ledger, &adapter, &ap);
    assert!(
        r.warnings.iter().any(|w| w.contains("stale")),
        "{:?}",
        r.warnings
    );

    // With the pin synced, no stale warning.
    let (dir2, adapter2, ledger2, approvals2) = make_repo();
    let r2 = check_charter(dir2.path(), &ledger2, &adapter2, &approvals2);
    assert!(
        !r2.warnings.iter().any(|w| w.contains("stale")),
        "{:?}",
        r2.warnings
    );
}

#[test]
fn an_advisory_gate_with_a_real_reason_is_valid() {
    // The advisory-reason rule must not fire when the reason IS present.
    let l: Ledger = serde_json::from_value(json!({
        "gates": [{
            "id": "a", "rule": "r", "tier": "T2", "checker": "c.mjs",
            "oracleSha256": "d", "blockingCondition": "advisory",
            "reason": "reports only while the flow stabilizes",
        }],
    }))
    .unwrap();
    assert_eq!(validate_ledger(&l), Vec::<String>::new());
}

#[test]
fn an_advisory_gate_without_a_reason_fails() {
    // The advisory-reason rule must FIRE when the reason is absent (kills the `!` delete).
    let l: Ledger = serde_json::from_value(json!({
        "gates": [{
            "id": "a", "rule": "r", "tier": "T2", "checker": "c.mjs",
            "oracleSha256": "d", "blockingCondition": "advisory",
        }],
    }))
    .unwrap();
    assert!(
        validate_ledger(&l)
            .iter()
            .any(|f| f.contains("advisory") && f.contains("non-empty \"reason\"")),
        "{:?}",
        validate_ledger(&l)
    );
}
