//! Empirical, deterministic proofs. These do not test internals in isolation; they
//! drive the real `crucible` binary end to end against real captured tool output and
//! real subprocesses, so "Crucible catches X" is a fact this suite reproduces on every
//! run, not a story in a README. See docs/PROOFS.md.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_crucible");

fn crucible(args: &[&str], repo: &Path) -> Output {
    Command::new(BIN)
        .args(args)
        .arg("--repo")
        .arg(repo)
        .output()
        .expect("run crucible")
}

// PROOF 1 — the mutation gate catches a reward-hacked test against the real cargo-mutants
// bytes in tests/fixtures/cargo-mutants-real.txt. If a cargo-mutants upgrade changes the
// output format, this proof fails instead of the gate silently passing everything.
#[test]
fn proof_harden_catches_real_cargo_mutants_survivor() {
    let fixture = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cargo-mutants-real.txt"
    ))
    .unwrap();
    assert!(
        fixture.contains("MISSED   src/lib.rs"),
        "the captured output really contains a missed mutant"
    );

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // A fresh repo with no commits: harden's diff scoping cannot resolve the base, so
    // it fails closed to high-risk and must block.
    Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(root)
        .status()
        .unwrap();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(root.join("mutants.txt"), &fixture).unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"repo":"proof","highRiskUnits":["lib"]}"#,
    )
    .unwrap();
    fs::write(
        root.join(".crucible/mutation.json"),
        r#"{"cmd":"cat mutants.txt","base":"HEAD"}"#,
    )
    .unwrap();

    let out = crucible(&["harden"], root);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("src/lib.rs:4"), "{stdout}");
    assert!(
        stdout.contains("replace should_buy -> bool with true"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Write a test that fails under this mutation"),
        "{stdout}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a high-risk survivor must block: {stdout}"
    );
}

// PROOF 2 — the reality arm catches a boot crash a green unit suite is blind to, using
// a REAL subprocess that either reaches ready or panics on startup. Unix-only: it drives
// a small sh script, which is exactly the shell the reality arm uses on unix.
#[cfg(unix)]
#[test]
fn proof_run_reports_runs_healthy_and_broken_on_boot_crash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(
        root.join(".crucible/acceptance.json"),
        r#"{
          "repo": "proof",
          "build": { "cmd": "true" },
          "boot": {
            "cmd": "if [ -n \"$CRUCIBLE_PROOF_BUG\" ]; then echo \"thread main panicked: DB migration failed\" >&2; exit 101; else echo app-ready; fi",
            "oracle": { "stdoutMatch": "app-ready", "stdoutForbid": "panicked" }
          },
          "drive": [{ "name": "checkout", "cmd": "echo ORDER PLACED", "oracle": { "stdoutMatch": "ORDER PLACED" } }]
        }"#,
    )
    .unwrap();

    let healthy = crucible(&["run"], root);
    let hs = String::from_utf8_lossy(&healthy.stdout);
    assert_eq!(healthy.status.code(), Some(0), "healthy app RUNS: {hs}");
    assert!(hs.contains("the app actually runs"), "{hs}");

    let crashed = Command::new(BIN)
        .arg("run")
        .arg("--repo")
        .arg(root)
        .env("CRUCIBLE_PROOF_BUG", "1")
        .output()
        .unwrap();
    let cs = String::from_utf8_lossy(&crashed.stdout);
    assert_eq!(
        crashed.status.code(),
        Some(1),
        "a boot crash is BROKEN: {cs}"
    );
    assert!(cs.contains("crashed on launch"), "{cs}");
    assert!(cs.contains("panicked"), "{cs}");
}

// PROOF 3 — the tamper-evidence catches an agent weakening the enforcement itself. Four
// distinct attacks, each a real edit to a real fixture repo, each caught, and honest
// again once reverted.
#[test]
fn proof_check_green_on_honest_gate_and_fails_on_every_weakening() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("checks")).unwrap();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    let checker = root.join("checks/check-thing.mjs");
    let runner = root.join("run.sh");
    let honest = "process.exit(0) // rejects unwrap() in money code\n";
    fs::write(&checker, honest).unwrap();
    fs::write(&runner, "node checks/check-thing.mjs\n").unwrap();
    let adapter = r#"{"repo":"tamper","charter":".crucible/charter.json","approvals":".crucible/approvals.json","gateRunner":{"file":"run.sh","checkerPattern":"node (checks/check-[a-z-]+\\.mjs)"},"highRiskUnits":["money"],"pinnedConfig":[".crucible/adapter.json"]}"#;
    fs::write(root.join(".crucible/adapter.json"), adapter).unwrap();
    fs::write(
        root.join(".crucible/charter.json"),
        r#"{"gates":[{"id":"thing","rule":"no unwrap() in money code","tier":"T1","checker":"checks/check-thing.mjs","blockingCondition":"always"}]}"#,
    )
    .unwrap();

    // Independent approvals via the real CLI.
    assert!(
        crucible(&["approve", "thing", "--by", "reviewer"], root)
            .status
            .success()
    );
    assert!(
        crucible(&["approve", "__config__", "--by", "reviewer"], root)
            .status
            .success()
    );

    let stderr = |o: Output| String::from_utf8_lossy(&o.stderr).into_owned();

    // Honest baseline.
    assert!(
        crucible(&["check"], root).status.success(),
        "the honest gate passes"
    );

    // Attack 1: weaken the checker.
    fs::write(&checker, "process.exit(0) // now a no-op\n").unwrap();
    let a1 = crucible(&["check"], root);
    assert_eq!(a1.status.code(), Some(1));
    assert!(
        stderr(a1).contains("not backed by an independent approval"),
        "weakened checker caught"
    );
    fs::write(&checker, honest).unwrap();

    // Attack 2: remove the gate from the required lane.
    fs::write(&runner, "# node checks/check-thing.mjs\n").unwrap();
    assert!(
        stderr(crucible(&["check"], root)).contains("not wired in the required lane"),
        "unwired gate caught"
    );
    fs::write(&runner, "node checks/check-thing.mjs\n").unwrap();

    // Attack 3: empty the high-risk list (pinned judge config).
    fs::write(
        root.join(".crucible/adapter.json"),
        adapter.replace("[\"money\"]", "[]"),
    )
    .unwrap();
    assert!(
        stderr(crucible(&["check"], root)).contains("judge config"),
        "weakened config caught"
    );
    fs::write(root.join(".crucible/adapter.json"), adapter).unwrap();

    // Attack 4: downgrade the gate T1 -> T2 while keeping its old approval.
    let charter = fs::read_to_string(root.join(".crucible/charter.json")).unwrap();
    fs::write(
        root.join(".crucible/charter.json"),
        charter.replace("\"tier\": \"T1\"", "\"tier\": \"T2\""),
    )
    .unwrap();
    assert!(
        stderr(crucible(&["check"], root)).contains("not backed by an independent approval"),
        "downgraded gate caught"
    );
    fs::write(root.join(".crucible/charter.json"), &charter).unwrap();

    // Restored: honest again.
    assert!(
        crucible(&["check"], root).status.success(),
        "reverting every attack returns to honest"
    );
}

// PROOF — a custom --recipe is a dry run and can never mint a certification. The
// reward-hack Codex verified: point a certifying arm at a throwaway recipe whose
// commands just echo the success markers. It may RUN, but it is explicitly NOT
// certified — no receipt is written and the CLI says so.
#[cfg(unix)]
#[test]
fn proof_custom_recipe_never_certifies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    // A throwaway recipe at a NON-canonical path whose "app" trivially passes.
    fs::write(
        root.join("sneaky.json"),
        r#"{
          "repo": "sneaky",
          "build": { "cmd": "true" },
          "boot": { "cmd": "echo app-ready", "oracle": { "stdoutMatch": "app-ready" } },
          "drive": [{ "name": "checkout", "cmd": "echo ORDER PLACED", "oracle": { "stdoutMatch": "ORDER PLACED" } }]
        }"#,
    )
    .unwrap();
    let out = Command::new(BIN)
        .arg("run")
        .arg("--recipe")
        .arg(root.join("sneaky.json"))
        .arg("--repo")
        .arg(root)
        .output()
        .unwrap();
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("NOT certified"),
        "a custom --recipe must announce it is a dry run, not a certification: {combined}"
    );
}
