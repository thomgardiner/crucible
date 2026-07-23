//! Empirical, deterministic proofs. These do not test internals in isolation; they
//! drive the real `crucible` binary end to end against real captured tool output and
//! real subprocesses, so "Crucible catches X" is a fact this suite reproduces on every
//! run, not a story in a README. See docs/PROOFS.md.
//!
//! Meta-rule: a proof that only matches a success banner without a material side-effect
//! (receipt, exit code, survivors file) is itself a reward-hack of this library. Every
//! certification claim asserts both the message and the artifact.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
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

/// Mirror of `hook::receipt_path` — integration tests cannot import the bin's modules.
fn receipt_path(repo: &Path, arm: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(repo.to_string_lossy().as_bytes());
    let dig = h.finalize();
    let mut key = String::with_capacity(dig.len() * 2);
    for b in dig {
        key.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        key.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    std::env::temp_dir()
        .join("crucible-receipts")
        .join(format!("{key}.{arm}.receipt"))
}

fn combined(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
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

    let receipt = receipt_path(root, "harden");
    let _ = fs::remove_file(&receipt);

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
    // Material artifact: survivors list for the next test to write — not just a banner.
    let survivors = fs::read_to_string(root.join(".crucible/survivors.json")).unwrap_or_default();
    assert!(
        survivors.contains("should_buy") || survivors.contains("src/lib.rs"),
        "survivors.json must name the live mutant, not be empty theater: {survivors}"
    );
    assert!(
        !receipt.exists(),
        "a blocking harden must not mint a success receipt"
    );
}

// PROOF 1b — positive control: the same pipeline with zero MISSED lines exits 0 and
// certifies. Without this, a broken harden that always exits 1 would still "pass" proof 1.
#[test]
fn proof_harden_clean_mutants_output_passes_and_certifies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .unwrap();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(
        root.join("mutants.txt"),
        "Found 2 mutants to test\nok       Unmutated baseline\n2 mutants tested: 0 missed, 2 caught\n",
    )
    .unwrap();
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

    let receipt = receipt_path(root, "harden");
    let _ = fs::remove_file(&receipt);

    let out = crucible(&["harden"], root);
    let s = combined(&out);
    assert_eq!(out.status.code(), Some(0), "clean mutants must pass: {s}");
    assert!(
        receipt.exists(),
        "canonical clean harden must mint a harden receipt at {}",
        receipt.display()
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

    let receipt = receipt_path(root, "run");
    let _ = fs::remove_file(&receipt);

    let healthy = crucible(&["run"], root);
    let hs = String::from_utf8_lossy(&healthy.stdout);
    assert_eq!(healthy.status.code(), Some(0), "healthy app RUNS: {hs}");
    assert!(hs.contains("the app actually runs"), "{hs}");
    assert!(
        receipt.exists(),
        "canonical healthy run must mint a run receipt"
    );

    let _ = fs::remove_file(&receipt);
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
    assert!(
        !receipt.exists(),
        "a broken run must not mint a success receipt"
    );
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
    fs::create_dir_all(root.join(".githooks")).unwrap();
    let checker = root.join("checks/check-thing.mjs");
    let runner = root.join("run.sh");
    let honest = "process.exit(0) // rejects unwrap() in money code\n";
    fs::write(&checker, honest).unwrap();
    fs::write(&runner, "node checks/check-thing.mjs\n").unwrap();
    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || exit 1\n",
    )
    .unwrap();
    let adapter = r#"{"repo":"tamper","charter":".crucible/charter.json","approvals":".crucible/approvals.json","gateRunner":{"file":"run.sh","checkerPattern":"node (checks/check-[a-z-]+\\.mjs)"},"highRiskUnits":["money"],"prePush":".githooks/pre-push","pinnedConfig":[".crucible/adapter.json"]}"#;
    fs::write(root.join(".crucible/adapter.json"), adapter).unwrap();
    fs::write(
        root.join(".crucible/charter.json"),
        r#"{"gates":[{"id":"thing","rule":"no unwrap() in money code","tier":"T1","checker":"checks/check-thing.mjs","blockingCondition":"always"}]}"#,
    )
    .unwrap();

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
    let receipt = receipt_path(root, "check");
    let _ = fs::remove_file(&receipt);

    assert!(
        crucible(&["check"], root).status.success(),
        "the honest gate passes"
    );
    assert!(
        receipt.exists(),
        "honest check must mint a check receipt"
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

    // Attack 5: strip pre-push (independence must be a verified fact, not a dead field).
    fs::write(
        root.join(".crucible/adapter.json"),
        adapter.replace(r#","prePush":".githooks/pre-push""#, ""),
    )
    .unwrap();
    // Adapter edit invalidates __config__; re-approve so the only failure is prePush.
    assert!(
        crucible(&["approve", "__config__", "--by", "reviewer"], root)
            .status
            .success()
    );
    let a5 = combined(&crucible(&["check"], root));
    assert!(
        a5.contains("prePush") || a5.contains("pre-push"),
        "missing prePush must fail check: {a5}"
    );
    fs::write(root.join(".crucible/adapter.json"), adapter).unwrap();
    assert!(
        crucible(&["approve", "__config__", "--by", "reviewer"], root)
            .status
            .success()
    );

    // Attack 6: pre-push exists but never runs crucible check.
    fs::write(root.join(".githooks/pre-push"), "#!/bin/sh\nexit 0\n").unwrap();
    let a6 = combined(&crucible(&["check"], root));
    assert!(
        a6.contains("does not run") || a6.contains("crucible check"),
        "inert pre-push must fail: {a6}"
    );
    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || exit 1\n",
    )
    .unwrap();

    assert!(
        crucible(&["check"], root).status.success(),
        "reverting every attack returns to honest"
    );
}

// PROOF 4 — a custom --recipe is a dry run and can never mint a certification.
// Asserts the banner AND the material side-effect: no receipt. Positive control: the
// same recipe content at the canonical path does mint a receipt.
#[cfg(unix)]
#[test]
fn proof_custom_recipe_never_certifies() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join(".crucible")).unwrap();
    let body = r#"{
          "repo": "sneaky",
          "build": { "cmd": "true" },
          "boot": { "cmd": "echo app-ready", "oracle": { "stdoutMatch": "app-ready" } },
          "drive": [{ "name": "checkout", "cmd": "echo ORDER PLACED", "oracle": { "stdoutMatch": "ORDER PLACED" } }]
        }"#;
    fs::write(root.join("sneaky.json"), body).unwrap();

    let receipt = receipt_path(root, "run");
    let _ = fs::remove_file(&receipt);

    let out = Command::new(BIN)
        .arg("run")
        .arg("--recipe")
        .arg(root.join("sneaky.json"))
        .arg("--repo")
        .arg(root)
        .output()
        .unwrap();
    let text = combined(&out);
    assert!(
        text.contains("NOT certified"),
        "a custom --recipe must announce it is a dry run: {text}"
    );
    // Even if the throwaway "app" would RUN, certification is refused.
    assert!(
        !receipt.exists(),
        "custom --recipe must not write a run receipt (got {})",
        receipt.display()
    );

    // Positive control: identical recipe at the canonical path certifies.
    fs::write(root.join(".crucible/acceptance.json"), body).unwrap();
    let _ = fs::remove_file(&receipt);
    let canon = crucible(&["run"], root);
    let cs = combined(&canon);
    assert_eq!(
        canon.status.code(),
        Some(0),
        "canonical recipe should RUN: {cs}"
    );
    assert!(
        receipt.exists(),
        "canonical recipe must mint a run receipt — otherwise the dry-run proof is unfalsifiable"
    );
}

// PROOF 5 — cover refuses to certify an empty scope (--base HEAD with a clean tree).
// A coverage command that would otherwise "succeed" must not mint a receipt when the
// diff is empty (the reward-hack: --base HEAD after the change is already committed).
#[test]
fn proof_cover_refuses_empty_scope() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .unwrap();
    for (k, v) in [("user.email", "p@x"), ("user.name", "p")] {
        let _ = Command::new("git")
            .args(["config", k, v])
            .current_dir(root)
            .status();
    }
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(root.join("src_lib.rs"), "pub fn f() {}\n").unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"repo":"proof","highRiskUnits":["lib"]}"#,
    )
    .unwrap();
    // Fake LCOV that would look "full" if we ever got that far.
    fs::create_dir_all(root.join("target")).unwrap();
    fs::write(
        root.join("target/lcov.info"),
        "TN:\nSF:src_lib.rs\nFN:1,f\nFNDA:1,f\nend_of_record\n",
    )
    .unwrap();
    fs::write(
        root.join(".crucible/coverage.json"),
        r#"{"cmd":"true","base":"HEAD","lcovPath":"target/lcov.info"}"#,
    )
    .unwrap();
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
            .args(["commit", "-q", "-m", "seed"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    // Clean worktree vs HEAD → empty changed set.

    let receipt = receipt_path(root, "cover");
    let _ = fs::remove_file(&receipt);

    let out = crucible(&["cover", "--base", "HEAD"], root);
    let s = combined(&out);
    assert_ne!(out.status.code(), Some(0), "empty scope must not pass: {s}");
    assert!(
        s.contains("empty scope") || s.contains("no changed files"),
        "must name empty-scope refusal: {s}"
    );
    assert!(!receipt.exists(), "empty-scope cover must not certify");
}

// PROOF 6 — this library's own proof/benchmark trees must not contain test-gaming smells.
// If we ship reward-hacked tests for the anti-reward-hacking tool, the product is a joke.
#[test]
fn proof_crucible_own_tests_have_no_test_gaming_smells() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = Command::new(BIN)
        .arg("test-smells")
        .arg("--repo")
        .arg(manifest)
        .arg("tests")
        .arg("src")
        .output()
        .unwrap();
    let s = combined(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "crucible's own suite must be smell-clean (fix the smell, do not weaken the scanner):\n{s}"
    );
}
