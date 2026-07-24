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
    let body = fs::read_to_string(&receipt).unwrap_or_default();
    assert!(
        body.starts_with("CRUCIBLE-RECEIPT-v1\nharden\n"),
        "receipt must be magic+arm bound, not an empty touch file: {body:?}"
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

    let receipt = receipt_path(root, "check");
    let _ = fs::remove_file(&receipt);

    assert!(
        crucible(&["check"], root).status.success(),
        "the honest gate passes"
    );
    assert!(receipt.exists(), "honest check must mint a check receipt");

    // Message alone is not enough: a warn-and-pass check would still "catch" every
    // attack below if we only searched stderr. Exit must be non-zero each time.
    let must_block = |label: &str, out: Output, needles: &[&str]| {
        let text = combined(&out);
        assert_ne!(
            out.status.code(),
            Some(0),
            "{label}: must fail closed, not warn-and-pass: {text}"
        );
        assert!(
            needles.iter().any(|n| text.contains(n)),
            "{label}: expected one of {needles:?} in: {text}"
        );
    };

    // Attack 1: weaken the checker.
    fs::write(&checker, "process.exit(0) // now a no-op\n").unwrap();
    must_block(
        "weakened checker",
        crucible(&["check"], root),
        &["not backed by an independent approval"],
    );
    fs::write(&checker, honest).unwrap();

    // Attack 2: remove the gate from the required lane.
    fs::write(&runner, "# node checks/check-thing.mjs\n").unwrap();
    must_block(
        "unwired gate",
        crucible(&["check"], root),
        &["not wired in the required lane"],
    );
    fs::write(&runner, "node checks/check-thing.mjs\n").unwrap();

    // Attack 3: empty the high-risk list (pinned judge config).
    fs::write(
        root.join(".crucible/adapter.json"),
        adapter.replace("[\"money\"]", "[]"),
    )
    .unwrap();
    must_block(
        "weakened config",
        crucible(&["check"], root),
        &["judge config"],
    );
    fs::write(root.join(".crucible/adapter.json"), adapter).unwrap();

    // Attack 4: downgrade the gate T1 -> T2 while keeping its old approval.
    let charter = fs::read_to_string(root.join(".crucible/charter.json")).unwrap();
    fs::write(
        root.join(".crucible/charter.json"),
        charter.replace("\"tier\": \"T1\"", "\"tier\": \"T2\""),
    )
    .unwrap();
    must_block(
        "downgraded gate",
        crucible(&["check"], root),
        &["not backed by an independent approval"],
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
    must_block(
        "missing prePush",
        crucible(&["check"], root),
        &["prePush", "pre-push"],
    );
    fs::write(root.join(".crucible/adapter.json"), adapter).unwrap();
    assert!(
        crucible(&["approve", "__config__", "--by", "reviewer"], root)
            .status
            .success()
    );

    // Attack 6: pre-push exists but never runs crucible check.
    fs::write(root.join(".githooks/pre-push"), "#!/bin/sh\nexit 0\n").unwrap();
    must_block(
        "inert pre-push",
        crucible(&["check"], root),
        &["does not run", "crucible check"],
    );

    // Attack 6b: text present, exit status swallowed — still not load-bearing.
    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || true\n",
    )
    .unwrap();
    must_block(
        "swallowed-exit pre-push",
        crucible(&["check"], root),
        &["does not run", "inert", "crucible check"],
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
    let body = fs::read_to_string(&receipt).unwrap_or_default();
    assert!(
        body.starts_with("CRUCIBLE-RECEIPT-v1\nrun\n"),
        "canonical receipt must be magic+arm bound: {body:?}"
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

// ---------------------------------------------------------------------------
// Contrast proofs: WITHOUT Crucible these look "done"; WITH Crucible they fail.
// Each pairs a green-normal signal with a Crucible block + material artifact.
// ---------------------------------------------------------------------------

fn git_init(root: &Path) {
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    for (k, v) in [
        ("user.email", "proof@crucible"),
        ("user.name", "proof"),
        ("commit.gpgsign", "false"),
        // Independence is push-time; local hooksPath must reach adapter.prePush.
        ("core.hooksPath", ".githooks"),
    ] {
        let _ = Command::new("git")
            .args(["config", k, v])
            .current_dir(root)
            .status();
    }
}

fn hook_stop(repo: &Path) -> Output {
    let payload = format!(
        r#"{{"cwd":{},"stop_hook_active":false}}"#,
        serde_json::to_string(&repo.to_string_lossy()).unwrap()
    );
    Command::new(BIN)
        .args(["hook", "stop"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut c| {
            use std::io::Write;
            c.stdin.take().unwrap().write_all(payload.as_bytes())?;
            c.wait_with_output()
        })
        .expect("hook stop")
}

// PROOF 7 — hollow tests that cargo test would mark green: assertion-free + assert!(true).
// Normal suite: green. Crucible test-smells: fails closed.
#[test]
fn proof_hollow_tests_are_green_under_cargo_but_fail_test_smells() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "hollow"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/lib.rs"),
        r#"
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    use super::*;

    // Looks like a test. Proves nothing.
    #[test]
    fn covers_add() {
        let _ = add(1, 1);
    }

    // Tautology: always green.
    #[test]
    fn always_true() {
        assert!(true);
    }
}
"#,
    )
    .unwrap();

    // WITHOUT Crucible: the suite is green — cargo test is happy.
    let cargo = Command::new("cargo")
        .args(["test", "-q"])
        .current_dir(root)
        .output()
        .expect("cargo test");
    assert_eq!(
        cargo.status.code(),
        Some(0),
        "control: cargo test must pass hollow suite:\n{}",
        combined(&cargo)
    );

    // WITH Crucible: test-smells names the gaming (absolute path — not the product tree).
    let hollow_src = root.join("src");
    let smells = Command::new(BIN)
        .arg("test-smells")
        .arg(&hollow_src)
        .arg("--repo")
        .arg(root)
        .output()
        .unwrap();
    let s = combined(&smells);
    assert_ne!(
        smells.status.code(),
        Some(0),
        "test-smells must fail closed on hollow suite: {s}"
    );
    assert!(
        s.contains("assertion") || s.contains("tautolog") || s.contains("assert!(true)"),
        "must name the hollow pattern: {s}"
    );
}

// PROOF 8 — never-called production function: a green LCOV for *other* symbols would
// not surface it; cover names it and refuses a receipt.
#[test]
fn proof_cover_blocks_never_called_high_risk_function() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/pay.rs"),
        "pub fn charge() {}\npub fn refund() {}\n",
    )
    .unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"repo":"proof","highRiskUnits":["pay"]}"#,
    )
    .unwrap();
    // LCOV: charge never hit, refund hit — a naive "suite green" story still holds for refund.
    fs::create_dir_all(root.join("target")).unwrap();
    fs::write(
        root.join("target/lcov.info"),
        "\
SF:src/pay.rs
FN:1,charge
FNDA:0,charge
FN:2,refund
FNDA:5,refund
end_of_record
",
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
    // Dirty change so cover has a non-empty scope.
    fs::write(
        root.join("src/pay.rs"),
        "pub fn charge() { /* changed */ }\npub fn refund() {}\n",
    )
    .unwrap();
    // Refresh LCOV mtime so it is not stale relative to this run.
    let _ = fs::write(
        root.join("target/lcov.info"),
        "\
SF:src/pay.rs
FN:1,charge
FNDA:0,charge
FN:2,refund
FNDA:5,refund
end_of_record
",
    );

    let receipt = receipt_path(root, "cover");
    let _ = fs::remove_file(&receipt);

    let out = crucible(&["cover", "--base", "HEAD"], root);
    let s = combined(&out);
    assert_ne!(
        out.status.code(),
        Some(0),
        "never-called high-risk must block: {s}"
    );
    assert!(
        s.contains("charge") || s.contains("never"),
        "must name the never-called function: {s}"
    );
    assert!(!receipt.exists(), "blocking cover must not mint a receipt");
}

// PROOF 9 — check-only verification is not enough to finish dirty work.
// An agent that only runs `crucible check` still gets blocked on Stop.
#[cfg(unix)]
#[test]
fn proof_check_only_receipt_does_not_clear_stop_nudge() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
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
    // Dirty work + check-only receipt (minted by a real successful check on a mini harness).
    fs::write(root.join("a.rs"), "fn a() { /* dirty */ }\n").unwrap();

    // Real check arm on a minimal honest gate so the receipt is written by production code.
    fs::create_dir_all(root.join("checks")).unwrap();
    fs::create_dir_all(root.join(".githooks")).unwrap();
    fs::write(root.join("checks/check-x.sh"), "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(
            root.join("checks/check-x.sh"),
            fs::Permissions::from_mode(0o755),
        );
    }
    fs::write(root.join("run.sh"), "sh checks/check-x.sh\n").unwrap();
    fs::write(
        root.join(".githooks/pre-push"),
        "#!/bin/sh\ncrucible check || exit 1\n",
    )
    .unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"repo":"nudge","charter":".crucible/charter.json","approvals":".crucible/approvals.json","gateRunner":{"file":"run.sh","checkerPattern":"sh (checks/check-[a-z-]+\\.sh)"},"highRiskUnits":["a"],"prePush":".githooks/pre-push","pinnedConfig":[".crucible/adapter.json"]}"#,
    )
    .unwrap();
    fs::write(
        root.join(".crucible/charter.json"),
        r#"{"gates":[{"id":"x","rule":"x","tier":"T1","checker":"checks/check-x.sh","blockingCondition":"always"}]}"#,
    )
    .unwrap();
    assert!(
        crucible(&["approve", "x", "--by", "rev"], root)
            .status
            .success()
    );
    assert!(
        crucible(&["approve", "__config__", "--by", "rev"], root)
            .status
            .success()
    );
    let check = crucible(&["check"], root);
    assert!(
        check.status.success(),
        "check itself must pass: {}",
        combined(&check)
    );
    assert!(
        receipt_path(root, "check").exists(),
        "check must mint a check receipt"
    );

    // Stop still blocks: check is not a verifying arm.
    let stop = hook_stop(root);
    let s = combined(&stop);
    assert!(
        s.contains("\"block\"") || s.contains("decision"),
        "Stop must block after check-only: {s}"
    );
    assert!(
        s.contains("harden") || s.contains("run") || s.contains("check alone"),
        "reason must say check is not enough: {s}"
    );
}

// PROOF 10 — a forged receipt (no magic) does not clear Stop.
#[cfg(unix)]
#[test]
fn proof_forged_receipt_does_not_clear_stop_nudge() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
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
    fs::write(root.join("a.rs"), "fn a() { /* dirty */ }\n").unwrap();

    // Casual echo forgery: timestamp only, no magic/arm.
    let p = receipt_path(root, "run");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    fs::write(&p, format!("{now}\n\n")).unwrap();

    let stop = hook_stop(root);
    let s = combined(&stop);
    assert!(
        s.contains("\"block\""),
        "forged receipt must not clear Stop: {s}"
    );
}

// PROOF 11 — "Found 0 mutants" is not "every mutant caught". No receipt.
#[test]
fn proof_zero_mutants_refuses_to_certify() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::write(
        root.join("mutants.txt"),
        "Found 0 mutants to test\n0 mutants tested: 0 missed, 0 caught\n",
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
    assert_ne!(
        out.status.code(),
        Some(0),
        "zero mutants must not pass: {s}"
    );
    assert!(
        s.contains("0 mutants") || s.contains("nothing was mutated"),
        "must name zero-mutant refusal: {s}"
    );
    assert!(
        !receipt.exists(),
        "zero-mutant harden must not mint a receipt"
    );
}

// PROOF 12 — live contrast on the committed mutation-crate: cargo test green,
// crucible harden (real captured cargo-mutants bytes) fails closed.
// This is the headline: a green unit suite is not proof tests bite.
#[test]
fn proof_live_mutation_crate_cargo_test_green_harden_blocks() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/proof/mutation-crate");
    assert!(
        crate_dir.join("Cargo.toml").exists(),
        "mutation-crate fixture missing"
    );

    let cargo = Command::new("cargo")
        .args(["test", "-q"])
        .current_dir(&crate_dir)
        .output()
        .expect("cargo test mutation-crate");
    assert_eq!(
        cargo.status.code(),
        Some(0),
        "control: mutation-crate must be green under cargo test:\n{}",
        combined(&cargo)
    );

    // Harden against the committed real cargo-mutants capture (same bytes as PROOF 1).
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    fs::create_dir_all(root.join(".crucible")).unwrap();
    let fixture = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cargo-mutants-real.txt"
    ))
    .unwrap();
    fs::write(root.join("mutants.txt"), fixture).unwrap();
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
    assert_ne!(
        out.status.code(),
        Some(0),
        "same green suite's missed mutant must block harden: {s}"
    );
    assert!(
        s.contains("should_buy") || s.contains("true"),
        "must name the reward-hack survivor: {s}"
    );
    let survivors = fs::read_to_string(root.join(".crucible/survivors.json")).unwrap_or_default();
    assert!(
        survivors.contains("should_buy") || survivors.contains("lib.rs"),
        "survivors.json material: {survivors}"
    );
    assert!(!receipt.exists(), "blocking harden mints no receipt");
}

// PROOF 13 — stale LCOV left on disk cannot certify a new cover run.
#[test]
fn proof_stale_lcov_cannot_certify_cover() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git_init(root);
    fs::create_dir_all(root.join(".crucible")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/pay.rs"), "pub fn charge() {}\n").unwrap();
    fs::write(
        root.join(".crucible/adapter.json"),
        r#"{"repo":"proof","highRiskUnits":["pay"]}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("target")).unwrap();
    let lcov = root.join("target/lcov.info");
    fs::write(
        &lcov,
        "SF:src/pay.rs\nFN:1,charge\nFNDA:1,charge\nend_of_record\n",
    )
    .unwrap();
    fs::write(
        root.join(".crucible/coverage.json"),
        // cmd that does not rewrite LCOV — leaves the stale file.
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
    fs::write(
        root.join("src/pay.rs"),
        "pub fn charge() { /* change */ }\n",
    )
    .unwrap();
    // Age past the 2s skew: LCOV mtime is now; sleep so the cover run starts later.
    std::thread::sleep(std::time::Duration::from_secs(3));

    let receipt = receipt_path(root, "cover");
    let _ = fs::remove_file(&receipt);
    let out = crucible(&["cover", "--base", "HEAD"], root);
    let s = combined(&out);
    assert_ne!(out.status.code(), Some(0), "stale LCOV must fail: {s}");
    assert!(
        s.contains("stale") || s.contains("older"),
        "must name stale LCOV: {s}"
    );
    assert!(!receipt.exists(), "stale cover must not certify");
}
