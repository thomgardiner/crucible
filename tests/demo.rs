//! Guards the committed example (examples/demo) against rot: if someone changes the
//! demo's checker without re-approving, or breaks its app, these fail. The demo is part
//! of the pitch, so it has to keep working. Unix-only: the demo app is a sh script.
#![cfg(unix)]

use std::path::Path;
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

/// Seed a local git history so harden can scope B..C (demo ships without a nested .git).
fn ensure_demo_git() {
    let demo = Path::new(DEMO);
    if demo.join(".git").exists() {
        return;
    }
    let git = |args: &[&str]| {
        assert!(
            Command::new("git")
                .args(args)
                .current_dir(demo)
                .status()
                .unwrap()
                .success(),
            "git {args:?}"
        );
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "demo@crucible.test"]);
    git(&["config", "user.name", "demo"]);
    // Two commits so HEAD~1..HEAD is a real range with high-risk units.
    git(&["add", "-A"]);
    git(&["-c", "commit.gpgsign=false", "commit", "-qm", "demo base"]);
    // Touch core so high-risk "core" is in the tip range when base is HEAD~1.
    let core = demo.join("app/core.ts");
    let mut body = std::fs::read_to_string(&core).unwrap();
    if !body.contains("// scope-pin") {
        body.push_str("\n// scope-pin\n");
        std::fs::write(&core, body).unwrap();
    }
    git(&["add", "app/core.ts"]);
    git(&[
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "core in scope",
    ]);
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
    ensure_demo_git();
    // Scope against the seed parent so app/core.ts is in B..C high-risk.
    let o = crucible(&["harden", "--base", "HEAD~1", "--candidate", "HEAD"]);
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    // Banner alone is theater: a print-always-exit-0 harden would still pass that.
    assert_ne!(
        o.status.code(),
        Some(0),
        "demo survivor must fail closed: {s}"
    );
    assert!(s.contains("surviving mutant"), "{s}");
    assert!(
        s.contains("Write a test that fails under this mutation"),
        "{s}"
    );
    let survivors =
        std::fs::read_to_string(format!("{DEMO}/.crucible/survivors.json")).unwrap_or_default();
    assert!(
        survivors.contains("shouldBuy") || survivors.contains("core") || survivors.contains("true"),
        "survivors.json must name the live mutant, not be empty: {survivors}"
    );
}
