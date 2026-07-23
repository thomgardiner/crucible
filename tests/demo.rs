//! Guards the committed example (examples/demo) against rot: if someone changes the
//! demo's checker without re-approving, or breaks its app, these fail. The demo is part
//! of the pitch, so it has to keep working. Unix-only: the demo app is a sh script.
#![cfg(unix)]

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_crucible");
const DEMO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo");

fn crucible(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .arg("--repo")
        .arg(DEMO)
        .output()
        .unwrap()
}

#[test]
fn demo_check_passes() {
    let o = crucible(&["check"]);
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn demo_run_reports_runs() {
    let o = crucible(&["run"]);
    let s = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains("the app actually runs"), "{s}");
}

#[test]
fn demo_harden_surfaces_the_survivor() {
    let o = crucible(&["harden"]);
    let s = String::from_utf8_lossy(&o.stdout);
    assert!(s.contains("surviving mutant"), "{s}");
    assert!(
        s.contains("Write a test that fails under this mutation"),
        "{s}"
    );
}
