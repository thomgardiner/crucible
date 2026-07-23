use super::*;
use crate::config::{Adapter, Ledger, load_json};
use std::fs;

fn starter_names() -> Vec<String> {
    starters("x").iter().map(|(n, _)| n.to_string()).collect()
}

fn all_scaffold_names() -> Vec<String> {
    let mut n = starter_names();
    n.push(SMOKE_CHECKER.into());
    n.push(GATE_RUNNER.into());
    n.push(".githooks/pre-push".into());
    n
}

#[test]
fn scaffold_writes_every_starter_into_a_fresh_repo() {
    let dir = tempfile::tempdir().unwrap();
    let r = scaffold(dir.path(), false).unwrap();
    let mut written = r.written.clone();
    written.sort();
    let mut expected = all_scaffold_names();
    expected.sort();
    assert_eq!(written, expected);
    assert!(r.skipped.is_empty());
    for name in starter_names() {
        assert!(
            dir.path().join(".crucible").join(&name).exists(),
            "{name} written"
        );
    }
    assert!(dir.path().join(SMOKE_CHECKER).exists());
    assert!(dir.path().join(GATE_RUNNER).exists());
    let hook = fs::read_to_string(dir.path().join(".githooks/pre-push")).unwrap();
    assert!(
        hook.contains("crucible check"),
        "pre-push must run crucible check"
    );
    let adapter: Adapter = load_json(&dir.path().join(".crucible/adapter.json")).unwrap();
    assert_eq!(
        adapter.gate_runner.file, "scripts/verify.sh",
        "fresh init must wire a real gate runner path"
    );
}

#[test]
fn scaffolded_config_parses_and_loads() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), false).unwrap();
    let base = dir.path().join(".crucible");
    let adapter: Adapter = load_json(&base.join("adapter.json")).unwrap();
    assert_eq!(adapter.charter, ".crucible/charter.json");
    assert!(!adapter.pinned_config.is_empty());
    let charter: Ledger = load_json(&base.join("charter.json")).unwrap();
    assert!(!charter.gates.is_empty());
    assert_eq!(charter.gates[0].id, "smoke");
    let approvals: Vec<serde_json::Value> = load_json(&base.join("approvals.json")).unwrap();
    assert!(approvals.is_empty());
}

#[test]
fn scaffold_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), false).unwrap();
    let marker = dir.path().join(".crucible/adapter.json");
    fs::write(&marker, "{\"repo\":\"already-edited\"}\n").unwrap();
    let r = scaffold(dir.path(), false).unwrap();
    assert!(r.written.is_empty());
    let mut skipped = r.skipped.clone();
    skipped.sort();
    let mut expected = all_scaffold_names();
    expected.sort();
    assert_eq!(skipped, expected);
    assert!(
        fs::read_to_string(&marker)
            .unwrap()
            .contains("already-edited")
    );
}

#[test]
fn scaffold_force_overwrites() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), false).unwrap();
    let marker = dir.path().join(".crucible/adapter.json");
    fs::write(&marker, "{\"repo\":\"already-edited\"}\n").unwrap();
    let r = scaffold(dir.path(), true).unwrap();
    assert_eq!(r.written.len(), all_scaffold_names().len());
    assert!(
        !fs::read_to_string(&marker)
            .unwrap()
            .contains("already-edited")
    );
}

#[test]
fn scaffold_uses_directory_name_as_repo() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("cool-service");
    fs::create_dir(&root).unwrap();
    scaffold(&root, false).unwrap();
    let adapter: Adapter = load_json(&root.join(".crucible/adapter.json")).unwrap();
    assert_eq!(adapter.repo.as_deref(), Some("cool-service"));
}
