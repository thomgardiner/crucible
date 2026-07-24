//! Guards the committed example (examples/demo) against rot: if someone changes the
//! demo's checker without re-approving, or breaks its app, these fail. The demo is part
//! of the pitch, so it has to keep working. Unix-only: the demo app is a sh script.
//!
//! All mutations run on a temporary copy of the demo tree so the repo fixtures stay clean.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const BIN: &str = env!("CARGO_BIN_EXE_crucible");
const DEMO_SRC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/demo");

struct Demo {
    _tmp: TempDir,
    root: PathBuf,
}

impl Demo {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("demo");
        copy_dir(Path::new(DEMO_SRC), &root);
        // Nested .git from a prior local run must not travel into the temp copy.
        let _ = std::fs::remove_dir_all(root.join(".git"));
        Self { _tmp: tmp, root }
    }

    fn crucible(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .args(args)
            .arg("--repo")
            .arg(&self.root)
            .output()
            .unwrap()
    }

    fn ensure_git_for_harden(&self) {
        let demo = &self.root;
        let git = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(demo)
                .status()
                .unwrap();
            assert!(st.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["config", "user.email", "demo@crucible.test"]);
        git(&["config", "user.name", "demo"]);
        git(&["add", "-A"]);
        git(&["-c", "commit.gpgsign=false", "commit", "-qm", "demo base"]);
        let core = demo.join("app/core.ts");
        let body = std::fs::read_to_string(&core).expect("demo core.ts");
        std::fs::write(
            &core,
            format!("{body}\n// scope-pin {}\n", std::process::id()),
        )
        .unwrap();
        git(&["add", "app/core.ts"]);
        git(&[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "core in scope",
        ]);
    }
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry.unwrap();
        let rel = entry.path().strip_prefix(src).unwrap();
        if rel.as_os_str().is_empty() {
            continue;
        }
        // Skip nested git metadata and generated crucible state.
        if rel.components().any(|c| {
            matches!(
                c.as_os_str().to_str(),
                Some(".git") | Some("node_modules") | Some("target")
            )
        }) {
            continue;
        }
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target).unwrap();
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

#[test]
fn demo_check_passes() {
    let demo = Demo::new();
    let o = demo.crucible(&["check"]);
    assert!(
        o.status.success(),
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn demo_run_reports_runs() {
    let demo = Demo::new();
    let o = demo.crucible(&["run"]);
    let s = String::from_utf8_lossy(&o.stdout);
    assert_eq!(o.status.code(), Some(0), "{s}");
    assert!(s.contains("the app actually runs"), "{s}");
}

#[test]
fn demo_harden_surfaces_the_survivor() {
    let demo = Demo::new();
    demo.ensure_git_for_harden();
    let o = demo.crucible(&["harden", "--base", "HEAD~1", "--candidate", "HEAD"]);
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
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
        std::fs::read_to_string(demo.root.join(".crucible/survivors.json")).unwrap_or_default();
    assert!(
        survivors.contains("shouldBuy") || survivors.contains("core") || survivors.contains("true"),
        "survivors.json must name the live mutant, not be empty: {survivors}"
    );
}
