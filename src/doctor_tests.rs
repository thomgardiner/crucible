use super::*;
use crate::init::scaffold;

#[test]
fn doctor_flags_a_repo_with_no_crucible_dir() {
    let dir = tempfile::tempdir().unwrap();
    let checks = doctor(dir.path());
    assert!(any_fail(&checks));
    assert!(
        checks
            .iter()
            .any(|c| c.msg.contains(".crucible/ not found"))
    );
}

#[test]
fn doctor_on_a_fresh_scaffold_parses_and_finds_gate_runner() {
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), false).unwrap();
    let checks = doctor(dir.path());
    assert!(
        checks
            .iter()
            .any(|c| c.status == Status::Pass && c.msg.contains("adapter.json parses"))
    );
    assert!(
        checks
            .iter()
            .any(|c| c.status == Status::Pass && c.msg.contains("gate runner resolves")),
        "fresh init must wire a real gate runner: {:?}",
        checks.iter().map(|c| &c.msg).collect::<Vec<_>>()
    );
    // Unapproved config/gates still fail the honesty check until `crucible approve`.
    assert!(
        any_fail(&checks),
        "unapproved scaffold must not read fully healthy"
    );
}

#[test]
fn doctor_flags_todo_commands_in_every_recipe_arm() {
    // A fresh scaffold's mutation/coverage/flake recipes all carry TODO commands; the
    // wiring check must name each arm, not just harden.
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), false).unwrap();
    let checks = doctor(dir.path());
    for (file, arm) in [
        ("mutation.json", "harden"),
        ("coverage.json", "cover"),
        ("flake.json", "flake"),
    ] {
        assert!(
            checks
                .iter()
                .any(|c| c.msg.contains(file) && c.msg.contains(arm)),
            "no TODO warning for {file}/{arm}"
        );
    }
}

#[test]
fn doctor_fails_on_a_malformed_or_cmdless_recipe() {
    // A recipe that exists but cannot run its arm is unhealthy, not silently skipped.
    let dir = tempfile::tempdir().unwrap();
    scaffold(dir.path(), false).unwrap();
    std::fs::write(dir.path().join(".crucible/coverage.json"), "{not json").unwrap();
    std::fs::write(dir.path().join(".crucible/flake.json"), "{}").unwrap();
    let checks = doctor(dir.path());
    assert!(
        checks
            .iter()
            .any(|c| c.status == Status::Fail && c.msg.contains("coverage.json does not parse")),
        "malformed recipe must fail"
    );
    assert!(
        checks
            .iter()
            .any(|c| c.status == Status::Fail && c.msg.contains("flake.json has no \"cmd\"")),
        "cmd-less recipe must fail"
    );
}
