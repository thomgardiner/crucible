//! End-to-end checks on CLI subcommands through the real binary, including the
//! path-with-spaces case that a naive shell-out would break and the flake arm's
//! fail-closed behavior on a red-but-deterministic suite.

use std::fs;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_crucible");

fn flake(recipe: &str) -> Output {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".crucible")).unwrap();
    fs::write(dir.path().join(".crucible/flake.json"), recipe).unwrap();
    Command::new(BIN)
        .arg("flake")
        .arg("--repo")
        .arg(dir.path())
        .output()
        .unwrap()
}

#[test]
fn flake_reports_a_deterministic_failure_as_not_passing() {
    // Codex round 2 #9: a suite that fails identically every run is deterministic but red.
    // It must exit non-zero, never earn a success receipt that suppresses the Stop nudge.
    let out = flake(r#"{ "cmd": "exit 1", "runs": 2 }"#);
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{s}");
    assert!(s.contains("failed every run"), "{s}");
}

#[test]
fn flake_catches_a_failure_printed_every_run_under_a_zero_exit() {
    // Codex round 3: a suite that prints a failing test every run but exits 0 must not pass.
    // The exit-code check alone misses it; the failing-every-run set catches it.
    let out = flake(
        r#"{ "cmd": "printf 'FAILED critical_test\n'", "runs": 2, "failPattern": "FAILED (\\w+)" }"#,
    );
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{s}");
    assert!(s.contains("failing in every run"), "{s}");
}

#[test]
fn flake_fails_closed_on_an_invalid_fail_pattern() {
    // Codex round 2 #4: an uncompilable failPattern used to be silently dropped, degrading
    // the check to exit-code-only while still claiming "stable". It must error instead.
    let out = flake(r#"{ "cmd": "true", "runs": 2, "failPattern": "[" }"#);
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(out.status.code(), Some(0), "{s}");
    assert!(s.contains("failPattern is not a valid regex"), "{s}");
}

#[test]
fn test_smells_runs_from_a_path_with_spaces_and_catches_a_plant() {
    let base = std::env::temp_dir().join("crucible smells test"); // space in the path
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    fs::write(
        base.join("bad.test.ts"),
        "it('x', () => { const y = 1; });\n",
    )
    .unwrap();

    let out = Command::new(BIN)
        .arg("test-smells")
        .arg(&base)
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{combined}");
    assert!(combined.contains("has no assertion"), "{combined}");

    let _ = fs::remove_dir_all(&base);
}

#[test]
fn test_smells_refuses_to_certify_a_scan_of_nothing() {
    // A dir with no test files scans nothing; "clean" over nothing is not clean.
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("notes.md"), "no tests here\n").unwrap();
    let out = Command::new(BIN)
        .arg("test-smells")
        .arg(dir.path())
        .output()
        .unwrap();
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(2), "{s}");
    assert!(s.contains("nothing was scanned"), "{s}");
}

#[test]
fn test_smells_clean_dir_exits_zero_and_scans_mjs() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("good.test.mjs"),
        "it('x', () => { expect(f()).toBe(1); });\n",
    )
    .unwrap();
    let out = Command::new(BIN)
        .arg("test-smells")
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn test_smells_errors_on_a_malformed_config() {
    // A config that exists but does not parse must be an error, not a silently
    // stricter scan (Codex round 6).
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".crucible")).unwrap();
    fs::write(dir.path().join(".crucible/test-smells.json"), "{not json").unwrap();
    fs::write(
        dir.path().join("ok.test.ts"),
        "it('x', () => { expect(f()).toBe(1); });\n",
    )
    .unwrap();
    let out = Command::new(BIN)
        .arg("--repo")
        .arg(dir.path())
        .arg("test-smells")
        .arg(dir.path())
        .output()
        .unwrap();
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(2), "{s}");
    assert!(s.contains("test-smells.json"), "{s}");
}

#[cfg(unix)]
#[test]
fn test_smells_follows_symlinked_test_trees() {
    // A symlinked test dir silently skipped is unverified code reading as clean.
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("shared-tests");
    let scanned = dir.path().join("tests");
    fs::create_dir_all(&real).unwrap();
    fs::create_dir_all(&scanned).unwrap();
    fs::write(
        real.join("hollow.test.ts"),
        "it('x', () => { const y = compute(); });\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&real, scanned.join("shared")).unwrap();
    let out = Command::new(BIN)
        .arg("test-smells")
        .arg(&scanned)
        .output()
        .unwrap();
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.status.code(), Some(1), "{s}");
    assert!(s.contains("has no assertion"), "{s}");
}

#[cfg(unix)]
#[test]
fn cover_fails_closed_when_the_command_floods_output() {
    // The disk guard must kill a runaway coverage command and the arm must refuse to
    // certify — not hang or fill the temp partition.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
    };
    git(&["init", "-q"]);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(root.join("src/a.rs"), "pub fn a() {}\n").unwrap();
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
    fs::write(root.join("src/a.rs"), "// touched\npub fn a() {}\n").unwrap();
    fs::write(
        root.join(".crucible/coverage.json"),
        r#"{"cmd":"yes crucible-flood","base":"HEAD"}"#,
    )
    .unwrap();

    let out = Command::new(BIN)
        .arg("--repo")
        .arg(root)
        .arg("cover")
        .output()
        .unwrap();
    let s = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_ne!(out.status.code(), Some(0), "{s}");
    assert!(s.contains("runaway output"), "{s}");
}

#[cfg(unix)]
#[test]
fn two_sessions_serialize_on_the_machine_wide_slot() {
    // The core fix: separate Crucible processes must not both run a heavy arm at once.
    // Session A holds the only slot while it "builds"; session B, arriving with no wait
    // budget, is refused rather than piling a second build onto the machine.
    let slots = tempfile::tempdir().unwrap();
    let mk = |cmd: &str| {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".crucible")).unwrap();
        fs::write(
            dir.path().join(".crucible/flake.json"),
            format!(r#"{{"cmd":"{cmd}","runs":2}}"#),
        )
        .unwrap();
        dir
    };
    let spawn = |dir: &std::path::Path, wait: &str| {
        let mut c = Command::new(BIN);
        c.arg("--repo")
            .arg(dir)
            .arg("flake")
            .env("CRUCIBLE_SLOTS_DIR", slots.path())
            .env("CRUCIBLE_MAX_CONCURRENCY", "1")
            .env("CRUCIBLE_SLOT_WAIT_SECS", wait);
        c
    };

    // A holds the slot for a couple of seconds.
    let a_dir = mk("sleep 2; echo ok");
    let mut a = spawn(a_dir.path(), "60").spawn().unwrap();

    // Give A time to acquire the slot, then B tries with no patience.
    std::thread::sleep(std::time::Duration::from_millis(700));
    let b_dir = mk("echo ok");
    let b = spawn(b_dir.path(), "0").output().unwrap();
    let b_out = format!(
        "{}{}",
        String::from_utf8_lossy(&b.stdout),
        String::from_utf8_lossy(&b.stderr)
    );
    assert_ne!(b.status.code(), Some(0), "B must be refused: {b_out}");
    assert!(
        b_out.contains("already active"),
        "B must fail on the slot gate, not something else: {b_out}"
    );

    // A finishes and frees the slot; a later session gets in.
    assert!(a.wait().unwrap().success());
    let c_dir = mk("echo ok");
    let c = spawn(c_dir.path(), "5").output().unwrap();
    assert_eq!(
        c.status.code(),
        Some(0),
        "C should acquire the freed slot: {}{}",
        String::from_utf8_lossy(&c.stdout),
        String::from_utf8_lossy(&c.stderr)
    );
}

#[test]
fn config_persists_the_machine_concurrency_and_names_the_live_source() {
    let cfg = tempfile::tempdir().unwrap();
    let set = Command::new(BIN)
        .args(["config", "max-concurrency", "2"])
        .env("CRUCIBLE_CONFIG_DIR", cfg.path())
        .env_remove("CRUCIBLE_MAX_CONCURRENCY")
        .output()
        .unwrap();
    let set_out = String::from_utf8_lossy(&set.stdout).to_string();
    assert_eq!(set.status.code(), Some(0), "{set_out}");
    let written = fs::read_to_string(cfg.path().join("config.json")).unwrap();
    assert!(written.contains("\"maxConcurrency\": 2"), "{written}");
    // Setting states the consequence: the per-tree memory ceiling under the new count.
    assert!(set_out.contains("per-tree memory ceiling"), "{set_out}");

    // Show resolves from the file and names it as the source.
    let show = Command::new(BIN)
        .args(["config"])
        .env("CRUCIBLE_CONFIG_DIR", cfg.path())
        .env_remove("CRUCIBLE_MAX_CONCURRENCY")
        .output()
        .unwrap();
    let show_out = String::from_utf8_lossy(&show.stdout).to_string();
    assert!(show_out.contains("config.json"), "{show_out}");

    // The env var still wins over the file, and show says so.
    let show_env = Command::new(BIN)
        .args(["config"])
        .env("CRUCIBLE_CONFIG_DIR", cfg.path())
        .env("CRUCIBLE_MAX_CONCURRENCY", "1")
        .output()
        .unwrap();
    let env_out = String::from_utf8_lossy(&show_env.stdout).to_string();
    assert!(env_out.contains("max-concurrency: 1"), "{env_out}");
    assert!(
        env_out.contains("env CRUCIBLE_MAX_CONCURRENCY"),
        "{env_out}"
    );

    // Zero is rejected at the parser, before it can reach the file.
    let zero = Command::new(BIN)
        .args(["config", "max-concurrency", "0"])
        .env("CRUCIBLE_CONFIG_DIR", cfg.path())
        .output()
        .unwrap();
    assert_ne!(zero.status.code(), Some(0));
}

#[cfg(unix)]
#[test]
fn the_admission_gate_honors_the_machine_config_file() {
    // The point of `crucible config`: a persisted slot count changes what the gate
    // admits, without any env var. With the file granting 2 slots, a second session is
    // admitted while the first still holds slot 0 — under the default (1) it is refused
    // (proven by two_sessions_serialize_on_the_machine_wide_slot).
    if std::thread::available_parallelism().map_or(1, |n| n.get()) < 2 {
        return; // the cores cap would clamp 2 back to 1 and invalidate the setup
    }
    let slots = tempfile::tempdir().unwrap();
    let cfg = tempfile::tempdir().unwrap();
    fs::write(cfg.path().join("config.json"), "{\"maxConcurrency\": 2}").unwrap();
    let mk = |cmd: &str| {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".crucible")).unwrap();
        fs::write(
            dir.path().join(".crucible/flake.json"),
            format!(r#"{{"cmd":"{cmd}","runs":2}}"#),
        )
        .unwrap();
        dir
    };
    let spawn = |dir: &std::path::Path, wait: &str| {
        let mut c = Command::new(BIN);
        c.arg("--repo")
            .arg(dir)
            .arg("flake")
            .env("CRUCIBLE_SLOTS_DIR", slots.path())
            .env("CRUCIBLE_CONFIG_DIR", cfg.path())
            .env_remove("CRUCIBLE_MAX_CONCURRENCY")
            .env("CRUCIBLE_SLOT_WAIT_SECS", wait);
        c
    };
    let a_dir = mk("sleep 2; echo ok");
    let mut a = spawn(a_dir.path(), "60").spawn().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(700));
    // B has no wait budget, so only the file's second slot can admit it.
    let b_dir = mk("echo ok");
    let b = spawn(b_dir.path(), "0").output().unwrap();
    assert_eq!(
        b.status.code(),
        Some(0),
        "B should be admitted by the file's second slot: {}{}",
        String::from_utf8_lossy(&b.stdout),
        String::from_utf8_lossy(&b.stderr)
    );
    assert!(a.wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn killing_crucible_reaps_the_spawned_build_tree() {
    // The orphan case: an agent kills a crucible session mid-run. Its spawned tree must
    // die with it, not keep eating RAM. The recipe records its own PID; after SIGTERM we
    // check that PID's *state* — gone or a zombie ("Z") means reaped, an alive state
    // ("S"/"R"/…) means it was orphaned. (A plain ps-grep is unreliable because a just-
    // killed process lingers briefly as a zombie that still matches its old command.)
    let slots = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pidfile = dir.path().join("child.pid");
    fs::create_dir_all(dir.path().join(".crucible")).unwrap();
    // Background a member (`sleep &`) in the same group, then exec into the leader. This
    // proves the WHOLE group is cleaned up, not just the leader — the backgrounded member
    // shares the leader's pgid and must die too.
    fs::write(
        dir.path().join(".crucible/flake.json"),
        format!(
            r#"{{"cmd":"echo $$ > {}; sleep 31017 & exec sleep 31017","runs":2}}"#,
            pidfile.display()
        ),
    )
    .unwrap();

    let mut child = Command::new(BIN)
        .arg("--repo")
        .arg(dir.path())
        .arg("flake")
        .env("CRUCIBLE_SLOTS_DIR", slots.path())
        .spawn()
        .unwrap();

    // Wait for the recipe to record the spawned leader's PID (it exec's into sleep, so this
    // PID is the process group's leader).
    let mut leader = None;
    for _ in 0..100 {
        if let Ok(s) = fs::read_to_string(&pidfile)
            && let Ok(pid) = s.trim().parse::<i32>()
        {
            leader = Some(pid);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let leader = leader.expect("the spawned build tree never started");

    // Count LIVE members of the leader's process group: `ps -axo pgid=,stat=`, rows whose
    // pgid == leader and whose state is not "Z" (a zombie is dead, just not yet reaped).
    let live_group_members = |pgid: i32| {
        let out = Command::new("/bin/ps")
            .args(["-axo", "pgid=,stat="])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| {
                let mut f = line.split_whitespace();
                let g: Option<i32> = f.next().and_then(|s| s.parse().ok());
                let stat = f.next().unwrap_or("");
                g == Some(pgid) && !stat.starts_with('Z')
            })
            .count()
    };
    // Leader + the backgrounded member should both be live before the kill.
    assert!(
        live_group_members(leader) >= 2,
        "the spawned group should have live members before the kill"
    );

    // Kill crucible with SIGTERM (the graceful "kill the agent" signal).
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let _ = child.wait();

    // The WHOLE group must be killed — no live (non-zombie) member of the leader's group
    // may survive. This proves group cleanup, not just leader disappearance.
    let mut reaped = false;
    for _ in 0..50 {
        if live_group_members(leader) == 0 {
            reaped = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !reaped {
        // Best-effort cleanup of the group so a failed test does not leak the sleep.
        let _ = Command::new("kill")
            .args(["-KILL", &format!("-{leader}")])
            .status();
    }
    assert!(reaped, "killing crucible orphaned its spawned build tree");
}
