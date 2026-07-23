//! Crucible CLI. Three arms, one binary:
//!
//!   crucible run          actually test the app: build, boot, drive the real
//!                         artifact; exit 0 only if it RUNS
//!   crucible harden       mutation gate: a mutant that survives on changed code
//!                         proves no test checks it; blocks on high-risk code
//!   crucible check        gate honesty: every declared gate is really wired and
//!                         no oracle drifted without an independent approval
//!
//! Supporting: `audit` (declared-vs-actual delta), `approve` (record an oracle
//! approval), `init` (scaffold `.crucible/`), `doctor` (adoption health),
//! `test-smells` (the shipped test-gaming checker).

mod admission;
mod approve;
mod cgroup;
mod charter;
mod config;
mod coverage;
mod doctor;
mod flake;
mod hash;
mod hook;
mod init;
mod mask;
mod mutation;
mod pre_push;
mod proc;
mod reality;
mod smells;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use config::{
    Adapter, Approval, CoverageRecipe, FlakeRecipe, Ledger, MutationRecipe, Recipe,
    TestSmellsConfig, Waiver, load_json,
};
use proc::Exec;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "crucible",
    version,
    about = "Honesty layer for your tests: check gates, run the real app, harden with mutation, catch hollow tests.",
    after_help = "Quick start:  crucible init && crucible doctor\nDocs:         https://github.com/thomgardiner/crucible#readme"
)]
struct Cli {
    /// Target repository (default: current directory).
    #[arg(long, global = true)]
    repo: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Scaffold .crucible/ config into a repo (idempotent).
    Init {
        /// Overwrite existing files.
        #[arg(long)]
        force: bool,
    },
    /// Gate honesty: every declared gate is really wired and no oracle drifted unapproved.
    Check,
    /// Print the declared-vs-actual enforcement delta.
    Audit,
    /// Build, boot, and drive the real app; exit 0 only if it RUNS.
    Run {
        #[arg(long)]
        json: bool,
        /// Recipe path (default: .crucible/acceptance.json).
        #[arg(long)]
        recipe: Option<PathBuf>,
    },
    /// Mutation gate: block when a mutant survives on high-risk changed code.
    Harden {
        /// Diff base B (default: the recipe's base, else origin/master).
        #[arg(long)]
        base: Option<String>,
        /// Candidate tip C (default: HEAD). Scopes arms to the explicit B..C range.
        #[arg(long)]
        candidate: Option<String>,
        /// Recipe path (default: .crucible/mutation.json).
        #[arg(long)]
        recipe: Option<PathBuf>,
    },
    /// Coverage floor: name the changed functions no test ever calls.
    Cover {
        /// Diff base B (default: the recipe's base, else origin/master).
        #[arg(long)]
        base: Option<String>,
        /// Candidate tip C (default: HEAD). Scopes arms to the explicit B..C range.
        #[arg(long)]
        candidate: Option<String>,
        /// Recipe path (default: .crucible/coverage.json).
        #[arg(long)]
        recipe: Option<PathBuf>,
    },
    /// Determinism check: run the suite N times and flag tests that flip pass/fail.
    Flake {
        /// Recipe path (default: .crucible/flake.json).
        #[arg(long)]
        recipe: Option<PathBuf>,
    },
    /// Record an independent approval of a gate's oracle (use __config__ for the judge config).
    Approve {
        gate: String,
        #[arg(long)]
        by: Option<String>,
        #[arg(long, default_value = "")]
        note: String,
    },
    /// Adoption health: is .crucible wired right?
    Doctor,
    /// Test-gaming checker: scan files/dirs for reward-hacked tests.
    #[command(name = "test-smells")]
    TestSmells {
        paths: Vec<PathBuf>,
        /// Assertion-helper names whose call counts as an assertion (comma-separated).
        /// Merged with `.crucible/test-smells.json`'s `assertionHelpers`.
        #[arg(long, value_delimiter = ',')]
        helpers: Vec<String>,
    },
    /// Show or set machine-level settings (concurrency slots for the heavy arms).
    Config {
        #[command(subcommand)]
        setting: Option<ConfigCmd>,
    },
    /// Internal: handle a TUI hook event (session-start | stop). Reads the payload on stdin.
    #[command(name = "hook", hide = true)]
    Hook { event: String },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// How many heavy runs (run/harden/cover/flake) may execute at once machine-wide.
    #[command(name = "max-concurrency")]
    MaxConcurrency {
        /// New slot count (>= 1, capped at core count); omit to show the current value.
        #[arg(value_parser = clap::value_parser!(u32).range(1..))]
        n: Option<u32>,
    },
}

fn main() -> ExitCode {
    // Reap spawned build trees if crucible is interrupted/terminated, so a killed session
    // never orphans a cargo tree that keeps eating RAM.
    proc::install_signal_cleanup();
    let cli = Cli::parse();
    let repo_root = cli
        .repo
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let result = match cli.cmd {
        Cmd::Init { force } => cmd_init(&repo_root, force),
        Cmd::Check => cmd_check(&repo_root),
        Cmd::Audit => cmd_audit(&repo_root),
        Cmd::Run { json, recipe } => cmd_run(&repo_root, json, recipe),
        Cmd::Harden {
            base,
            candidate,
            recipe,
        } => cmd_harden(&repo_root, base, candidate, recipe),
        Cmd::Cover {
            base,
            candidate,
            recipe,
        } => cmd_cover(&repo_root, base, candidate, recipe),
        Cmd::Flake { recipe } => cmd_flake(&repo_root, recipe),
        Cmd::Approve { gate, by, note } => cmd_approve(&repo_root, &gate, by, &note),
        Cmd::Doctor => cmd_doctor(&repo_root),
        Cmd::TestSmells { paths, helpers } => cmd_test_smells(&repo_root, &paths, &helpers),
        Cmd::Config { setting } => cmd_config(setting),
        Cmd::Hook { event } => cmd_hook(&event),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("crucible: {e:#}");
            ExitCode::from(2)
        }
    }
}

// ---- machine config --------------------------------------------------------

fn cmd_config(setting: Option<ConfigCmd>) -> Result<u8> {
    match setting {
        Some(ConfigCmd::MaxConcurrency { n: Some(n) }) => {
            let path = admission::set_max(n).map_err(anyhow::Error::msg)?;
            let cores = admission::cores();
            let effective = n.min(cores);
            println!("max-concurrency {n} → {}", path.display());
            if effective < n {
                println!("  capped to {effective} on this machine ({cores} cores)");
            }
            print_tree_budget(effective);
            if std::env::var("CRUCIBLE_MAX_CONCURRENCY").is_ok() {
                println!(
                    "  note: CRUCIBLE_MAX_CONCURRENCY is set in this shell and overrides the file"
                );
            }
            Ok(0)
        }
        // Bare `config` and bare `config max-concurrency` both show the current state.
        _ => {
            let (n, source) = admission::effective_max();
            let source = match source {
                admission::Source::Env => "env CRUCIBLE_MAX_CONCURRENCY".to_string(),
                admission::Source::ConfigFile => admission::config_path().display().to_string(),
                admission::Source::Default => {
                    "default — set with `crucible config max-concurrency <N>`".to_string()
                }
            };
            println!("max-concurrency: {n} ({source})");
            print_tree_budget(n);
            Ok(0)
        }
    }
}

// More slots = a lower ceiling per tree (the 80%-of-RAM heavy-work budget splits across
// the allowed concurrency), so the trade-off is printed wherever the slot count is shown.
fn print_tree_budget(slots: u32) {
    if let Some(bytes) = proc::default_memory_bytes(slots) {
        println!(
            "  per-tree memory ceiling: ~{:.1} GiB (80% of RAM across {slots} slot(s))",
            bytes as f64 / (1024.0 * 1024.0 * 1024.0)
        );
    }
}

// ---- gate arm -------------------------------------------------------------

fn load_gate_config(repo_root: &Path) -> Result<(Adapter, Ledger, Vec<Approval>)> {
    let adapter_path = repo_root.join(".crucible/adapter.json");
    if !adapter_path.exists() {
        bail!(
            "no adapter at .crucible/adapter.json — run `crucible init`, then see docs/ADOPTING.md"
        );
    }
    let adapter = load_json::<Adapter>(&adapter_path)?;
    let ledger = load_json::<Ledger>(&repo_root.join(&adapter.charter))?;
    let approvals =
        load_json::<Vec<Approval>>(&repo_root.join(&adapter.approvals)).unwrap_or_default();
    Ok((adapter, ledger, approvals))
}

fn cmd_check(repo_root: &Path) -> Result<u8> {
    let (adapter, ledger, approvals) = load_gate_config(repo_root)?;
    let r = charter::check_charter(repo_root, &ledger, &adapter, &approvals);
    for w in &r.warnings {
        eprintln!("  warn: {w}");
    }
    if r.failures.is_empty() {
        hook::write_receipt(repo_root, "check"); // only a passing verification counts
        println!(
            "Crucible: every gate is honest ({} gates, 0 violations).",
            ledger.gates.len()
        );
        Ok(0)
    } else {
        eprintln!("\nCrucible: {} gate violation(s):", r.failures.len());
        for f in &r.failures {
            eprintln!("  ✗ {f}");
        }
        Ok(1)
    }
}

fn cmd_audit(repo_root: &Path) -> Result<u8> {
    let (adapter, ledger, _) = load_gate_config(repo_root)?;
    let rep = charter::audit_charter(repo_root, &ledger, &adapter);
    println!("Enforcement tiers:");
    for (t, n) in &rep.counts {
        println!("  {t}: {n}");
    }
    if !rep.prose_only.is_empty() {
        println!("\nProse-only (T3, declared unenforced):");
        for p in &rep.prose_only {
            println!(
                "  • {} — {}\n      reason: {}",
                p.id,
                p.rule,
                p.reason.as_deref().unwrap_or("")
            );
        }
    }
    if !rep.undeclared.is_empty() {
        println!("\n⚠ Enforced but undeclared (wired checkers missing from the ledger):");
        for c in &rep.undeclared {
            println!("  • {c}");
        }
    }
    if !rep.high_risk_units.is_empty() {
        println!(
            "\nHigh-risk (money/checkout) units — mutation blocks here: {}",
            rep.high_risk_units.join(", ")
        );
    }
    Ok(0)
}

fn cmd_approve(repo_root: &Path, gate: &str, by: Option<String>, note: &str) -> Result<u8> {
    let adapter_path = repo_root.join(".crucible/adapter.json");
    if !adapter_path.exists() {
        bail!("no adapter at .crucible/adapter.json — run `crucible init` first");
    }
    let adapter: Adapter = load_json(&adapter_path)?;
    let by = by
        .or_else(|| std::env::var("USER").ok())
        .or_else(|| std::env::var("USERNAME").ok())
        .unwrap_or_else(|| "unknown".into());
    let fp = approve::approve(repo_root, &adapter, gate, &by, note)?;
    println!(
        "Crucible: approved \"{gate}\" → {}… (by {by}).",
        &fp[..fp.len().min(12)]
    );
    println!(
        "Commit this approval in a separate commit from the config/checker it blesses — \
         `crucible check` flags same-commit self-approvals. Independence is your pre-push \
         (verified wired) + that audit trail, not a second cryptographic identity."
    );
    Ok(0)
}

// ---- reality arm ----------------------------------------------------------

// A custom --recipe is a dry run, never a certification. The reward-hack Codex
// verified: point a certifying arm at a throwaway recipe whose build/boot/drive
// commands just echo the success markers, and it writes a passing receipt. Only
// the repo's canonical, approved recipe can mint a receipt.
fn certifies(repo_root: &Path, custom_recipe: bool, arm: &str) -> bool {
    if custom_recipe {
        eprintln!(
            "Crucible {arm}: a custom --recipe is a dry run — NOT certified (no receipt is written); only the repo's approved .crucible recipe can certify."
        );
        return false;
    }
    // If the repo has a full gate config, refuse receipts while judge config is dirty
    // (self-weakened acceptance/mutation/waivers without re-approval). Incomplete
    // scaffolds (proof fixtures with only mutation.json) still allow arm receipts.
    if repo_root.join(".crucible/adapter.json").exists()
        && repo_root.join(".crucible/charter.json").exists()
    {
        match load_gate_config(repo_root) {
            Ok((adapter, ledger, approvals)) => {
                let r = charter::check_charter(repo_root, &ledger, &adapter, &approvals);
                if !r.failures.is_empty() {
                    eprintln!(
                        "Crucible {arm}: judge config is not cleanly approved — NOT certified \
                         (fix `crucible check` / re-approve before a receipt can clear the Stop nudge):"
                    );
                    for f in r.failures.iter().take(5) {
                        eprintln!("  ✗ {f}");
                    }
                    return false;
                }
            }
            Err(e) => {
                eprintln!("Crucible {arm}: could not load gate config — NOT certified: {e}");
                return false;
            }
        }
    }
    true
}

fn cmd_run(repo_root: &Path, json: bool, recipe: Option<PathBuf>) -> Result<u8> {
    let custom_recipe = recipe.is_some();
    let recipe_path = recipe.unwrap_or_else(|| repo_root.join(".crucible/acceptance.json"));
    if !recipe_path.exists() {
        bail!(
            "no recipe at {} — create .crucible/acceptance.json (see the crucible skill)",
            recipe_path.display()
        );
    }
    let recipe: Recipe = load_json(&recipe_path)?;
    // Machine-wide gate: hold a slot for the whole build/boot/drive so concurrent
    // sessions do not collectively exhaust memory.
    let _slot = admission::acquire().map_err(anyhow::Error::msg)?;
    // Cap each step's process tree with the machine-aware default so a runaway app OOMs
    // itself, not the box.
    let memory_bytes = resolve_memory_limit(None)?;
    if memory_bytes.is_none() {
        eprintln!("Crucible: could not read total RAM — running the app without a memory ceiling.");
    }
    let report = reality::run_crucible(repo_root, &recipe, &proc::ShellExec, memory_bytes);
    if report.verdict == "RUNS" && certifies(repo_root, custom_recipe, "run") {
        hook::write_receipt(repo_root, "run"); // only a passing, canonical-recipe run counts
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", reality::format_report(&report));
    }
    Ok(if report.verdict == "RUNS" { 0 } else { 1 })
}

// ---- mutation arm ---------------------------------------------------------

// A change is high-risk if it touches any of the adapter's highRiskUnits. Fails
// closed in three ways, because every one of them was a way to certify a
// reward-hack: no declared units (nothing safely downgrades to advisory), an
// empty diff (a `--base HEAD` that scopes to nothing cannot prove the change is
// low-risk), or an undiscoverable diff — all mean "treat as high-risk so
// survivors block" rather than silently passing.
fn changed_hits_high_risk(
    repo_root: &Path,
    base: &str,
    candidate: &str,
    high_risk: &[String],
) -> bool {
    if high_risk.is_empty() {
        return true;
    }
    match changed_files(repo_root, base, candidate) {
        Ok(files) if files.is_empty() => true,
        Ok(files) => files
            .iter()
            .any(|f| high_risk.iter().any(|u| coverage::path_hits_unit(f, u))),
        Err(_) => true,
    }
}

fn cmd_harden(
    repo_root: &Path,
    base: Option<String>,
    candidate: Option<String>,
    recipe: Option<PathBuf>,
) -> Result<u8> {
    let custom_recipe = recipe.is_some();
    let recipe_path = recipe.unwrap_or_else(|| repo_root.join(".crucible/mutation.json"));
    if !recipe_path.exists() {
        bail!(
            "no mutation recipe at {} — create .crucible/mutation.json (see docs/METHODOLOGY.md)",
            recipe_path.display()
        );
    }
    let recipe: MutationRecipe = load_json(&recipe_path)?;
    if recipe.cmd.trim().is_empty() || recipe.cmd.starts_with("TODO") {
        bail!(
            "mutation.json cmd is not set — point it at a diff-scoped mutation command that emits survivor lines"
        );
    }
    let waivers_path = repo_root.join(".crucible/mutation-waivers.json");
    let waivers: Vec<Waiver> = load_json(&waivers_path).unwrap_or_default();
    let base = base
        .or_else(|| recipe.base.clone())
        .unwrap_or_else(|| "origin/master".into());
    let candidate = candidate.unwrap_or_else(|| "HEAD".into());

    // Scope B..C before running mutation:
    // - empty resolved scope → refuse (same as cover)
    // - unresolvable base (e.g. fresh git with no commits) → fail closed high-risk
    let high_risk_units = load_json::<Adapter>(&repo_root.join(".crucible/adapter.json"))
        .map(|a| a.high_risk_units)
        .unwrap_or_default();
    let is_high_risk = match changed_files(repo_root, &base, &candidate) {
        Ok(changed) if changed.is_empty() => {
            bail!(
                "no changed files on {base}...{candidate} — harden refuses to certify an empty scope (nothing mutated means nothing was proven)"
            );
        }
        Ok(_) => changed_hits_high_risk(repo_root, &base, &candidate, &high_risk_units),
        Err(_) => true, // cannot resolve B..C → treat as high-risk (historical fail-closed)
    };

    let cwd = match &recipe.cwd {
        Some(c) => repo_root.join(c),
        None => repo_root.to_path_buf(),
    };
    // Bind B..C into the recipe via {base}/{candidate} placeholders so the
    // mutation tool is forced onto the declared range (not only high-risk class).
    let mut recipe = recipe;
    recipe.cmd = recipe
        .cmd
        .replace("{base}", &base)
        .replace("{candidate}", &candidate);
    // The mutation run forks a full build/test matrix; hold a machine-wide slot so
    // several sessions do not run cargo-mutants at once and OOM the box.
    let _slot = admission::acquire().map_err(anyhow::Error::msg)?;
    let res = mutation::run_harden(&recipe, &cwd, &waivers, is_high_risk, &proc::ShellExec);
    if res.verdict == "pass" && certifies(repo_root, custom_recipe, "harden") {
        hook::write_receipt(repo_root, "harden"); // only a clean, canonical-recipe run counts
    }

    if !res.waiver_failures.is_empty() {
        eprintln!("Crucible harden: invalid mutation waivers:");
        for f in &res.waiver_failures {
            eprintln!("  ✗ {f}");
        }
        return Ok(1);
    }
    if let Some(e) = &res.error {
        eprintln!("Crucible harden: {e}");
    }

    // Every survivor is the next test to write; persist the report so an agent loop can
    // consume it.
    let survivors_json = serde_json::to_string_pretty(&res.report)? + "\n";
    let _ = std::fs::write(repo_root.join(".crucible/survivors.json"), survivors_json);

    if !res.report.is_empty() {
        println!(
            "Crucible harden: {} surviving mutant(s) on changed code — no test catches these:",
            res.report.len()
        );
        for item in &res.report {
            println!("  ✗ {}", item.instruction);
        }
    }
    let tier = if is_high_risk {
        "high-risk (blocking)"
    } else {
        "advisory"
    };
    match res.verdict.as_str() {
        "pass" => {
            println!("Crucible harden: every mutant on changed code is caught or waived ({tier}).")
        }
        "advisory" => println!(
            "Crucible harden: survivors above are advisory here; harden the tests before this reaches high-risk code."
        ),
        _ => {}
    }
    Ok(if res.verdict == "block" { 1 } else { 0 })
}

// The effective per-tree memory ceiling in bytes: the recipe's explicit `memoryMb` when
// set (validated, non-zero), otherwise a machine-aware default that composes with the
// concurrency gate so heavy trees can never collectively OOM the box. Returns None only
// when a recipe sets no limit AND total RAM cannot be read — a rare, reported fallback.
fn resolve_memory_limit(memory_mb: Option<u64>) -> Result<Option<u64>> {
    match memory_mb {
        Some(mb) => Ok(Some(
            proc::memory_limit_bytes(mb).map_err(anyhow::Error::msg)?,
        )),
        None => Ok(proc::default_memory_bytes(admission::max_concurrency())),
    }
}

// Run a recipe command under the shared resource guards: a hard timeout, the unconditional
// output/disk cap, and a process-tree memory ceiling (explicit or machine-aware default).
fn run_recipe_command(
    cmd: &str,
    cwd: &Path,
    timeout: std::time::Duration,
    memory_mb: Option<u64>,
) -> Result<proc::Output> {
    Ok(match resolve_memory_limit(memory_mb)? {
        Some(bytes) => proc::ShellExec.run_limited(cmd, cwd, timeout, bytes),
        None => {
            eprintln!(
                "Crucible: could not read total RAM — running without a memory ceiling; set memoryMb to cap it."
            );
            proc::ShellExec.run(cmd, cwd, timeout)
        }
    })
}

// ---- coverage floor -------------------------------------------------------

// A failed diff (bad base, not a repo) must be an error, not an empty set: an empty set
// scopes the floor to nothing, which reads as fully covered. A successful diff with no
// output is genuinely "no changes" and stays Ok(empty). Untracked files are part of the
// change too — `git diff` cannot see a brand-new file, which is exactly the least-tested
// code. When `--repo` is a subdirectory of a larger git worktree, only paths under that
// adoption root count (monorepo dirt outside the demo must not flip high-risk scoping).
/// Files changed on the exclusive B..C range (plus untracked when C is HEAD).
/// Callers pass explicit base/candidate so empty HEAD..HEAD cannot falsely certify.
fn changed_files(repo_root: &Path, base: &str, candidate: &str) -> Result<HashSet<String>> {
    // Diff-discovery runs BEFORE the machine-wide slot is acquired, so a hung or hostile
    // git must not hang Crucible outside every resource cap.
    let run = |args: &[&str]| -> Result<Vec<String>> {
        let out =
            proc::run_program_bounded("git", args, repo_root, std::time::Duration::from_secs(60))
                .context("running git")?;
        if out.timed_out {
            bail!(
                "`git {}` timed out after 60s — a hung git blocked diff discovery",
                args.join(" ")
            );
        }
        if out.code != 0 {
            bail!("`git {}` failed: {}", args.join(" "), out.stderr.trim());
        }
        Ok(out
            .stdout
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect())
    };
    // Explicit B..C: live tip includes working-tree dirt against B; pinned C
    // uses the exclusive committed range only.
    let candidate_is_head = candidate == "HEAD"
        || run(&["rev-parse", "--verify", candidate])
            .ok()
            .and_then(|v| v.into_iter().next())
            == run(&["rev-parse", "HEAD"])
                .ok()
                .and_then(|v| v.into_iter().next());
    let mut files: HashSet<String> = if candidate_is_head {
        run(&["diff", "--no-renames", "--name-only", base])?
            .into_iter()
            .collect()
    } else {
        let range = format!("{base}...{candidate}");
        run(&["diff", "--no-renames", "--name-only", &range])?
            .into_iter()
            .collect()
    };
    if candidate_is_head {
        files.extend(run(&["ls-files", "--others", "--exclude-standard"])?);
    }

    // Git prints paths relative to the worktree root, not to `--repo`. Scope and re-root.
    let toplevel = run(&["rev-parse", "--show-toplevel"])?
        .into_iter()
        .next()
        .unwrap_or_default();
    if toplevel.is_empty() {
        return Ok(files);
    }
    let top = PathBuf::from(toplevel.trim());
    let repo_abs = repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf());
    let top_abs = top.canonicalize().unwrap_or(top);
    let Ok(rel_root) = repo_abs.strip_prefix(&top_abs) else {
        return Ok(files); // repo_root is not inside this worktree — leave paths alone
    };
    if rel_root.as_os_str().is_empty() {
        return Ok(files); // adoption root is the git root
    }
    let pref = rel_root.to_string_lossy().replace('\\', "/");
    let pref = pref.trim_start_matches("./");
    let mut scoped = HashSet::new();
    for f in files {
        let f = f.replace('\\', "/");
        if let Some(rest) = f.strip_prefix(&format!("{pref}/")) {
            scoped.insert(rest.to_string());
        }
    }
    Ok(scoped)
}

fn cmd_cover(
    repo_root: &Path,
    base: Option<String>,
    candidate: Option<String>,
    recipe: Option<PathBuf>,
) -> Result<u8> {
    let custom_recipe = recipe.is_some();
    let recipe_path = recipe.unwrap_or_else(|| repo_root.join(".crucible/coverage.json"));
    if !recipe_path.exists() {
        bail!(
            "no coverage recipe at {} — create .crucible/coverage.json (a command that emits LCOV)",
            recipe_path.display()
        );
    }
    let recipe: CoverageRecipe = load_json(&recipe_path)?;
    if recipe.cmd.trim().is_empty() || recipe.cmd.starts_with("TODO") {
        bail!(
            "coverage.json cmd is not set — point it at a command that emits LCOV, e.g. `cargo llvm-cov --lcov`"
        );
    }
    let base = base
        .or_else(|| recipe.base.clone())
        .unwrap_or_else(|| "origin/master".into());
    let candidate = candidate.unwrap_or_else(|| "HEAD".into());
    let mut changed = changed_files(repo_root, &base, &candidate).with_context(|| {
        format!("cannot scope the coverage floor to the diff {base}...{candidate}")
    })?;
    // A deleted file has no functions to cover; keeping it would demand coverage records
    // for code that no longer exists (Codex round 5).
    changed.retain(|f| repo_root.join(f).exists());
    // An empty scope must never certify: `cover --base HEAD` (or a base that
    // already contains the change) scopes the floor to nothing, which would read
    // as "fully covered" and write a receipt for a change it never inspected.
    if changed.is_empty() {
        bail!(
            "no changed files on {base}...{candidate} — cover refuses to certify an empty scope (nothing to cover means nothing was proven, not that the floor is met)"
        );
    }
    let high_risk = load_json::<Adapter>(&repo_root.join(".crucible/adapter.json"))
        .map(|a| a.high_risk_units)
        .unwrap_or_default();
    let cwd = match &recipe.cwd {
        Some(c) => repo_root.join(c),
        None => repo_root.to_path_buf(),
    };
    let timeout = std::time::Duration::from_secs(recipe.timeout_sec.unwrap_or(1800));
    // The coverage build is heavy; gate it machine-wide before spawning.
    let _slot = admission::acquire().map_err(anyhow::Error::msg)?;
    let started = std::time::SystemTime::now();
    let out = run_recipe_command(&recipe.cmd, &cwd, timeout, recipe.memory_mb)?;
    // Fail closed: a coverage tool that timed out or exited non-zero produced no trustworthy
    // report, and "no evidence" must never read as "fully covered".
    if out.timed_out {
        bail!("coverage command timed out — no coverage report produced");
    }
    if out.memory_exceeded {
        bail!("coverage command exceeded its memory limit — no coverage report produced");
    }
    if out.output_exceeded {
        bail!("coverage command produced runaway output and was killed to protect the disk");
    }
    // A non-zero exit means this run did not finish cleanly, so any LCOV on disk may be
    // stale from a previous run — fail closed regardless of lcovPath (Codex round 3).
    if out.code != 0 {
        bail!(
            "coverage command exited {} — the run did not complete, so its coverage cannot be trusted (a lcovPath file may be stale)",
            out.code
        );
    }
    let lcov_text = match &recipe.lcov_path {
        Some(p) => {
            let path = repo_root.join(p);
            let meta = std::fs::metadata(&path).with_context(|| {
                format!("reading LCOV at {p} (did the coverage command write it?)")
            })?;
            // Reject a report that predates this run (stale file left on disk). Allow 2s
            // of filesystem mtime skew so coarse timestamps do not false-fail.
            if let Ok(mtime) = meta.modified() {
                let skew = std::time::Duration::from_secs(2);
                if mtime + skew < started {
                    bail!(
                        "LCOV at {p} is older than this coverage run — refusing a stale report \
                         (agent could leave a green prior file and skip a real re-run)"
                    );
                }
            }
            std::fs::read_to_string(&path).with_context(|| format!("reading LCOV at {p}"))?
        }
        None => out.output,
    };
    let files = coverage::parse_lcov(&lcov_text);
    if files.is_empty() {
        bail!(
            "no LCOV records parsed — the coverage command produced no coverage data; cannot certify coverage"
        );
    }
    let report = coverage::cover(&files, &changed, &high_risk);
    // No evidence must never read as full coverage (Codex round 3): a changed source file
    // with no LCOV record at all is untested code the intersection would skip, and matched
    // records carrying zero FN data make "every changed function is exercised" vacuous.
    if !report.unmatched_changed.is_empty() {
        println!(
            "Crucible cover: {} changed source file(s) have no coverage record at all — no test run ever compiled them:",
            report.unmatched_changed.len()
        );
        for f in &report.unmatched_changed {
            println!("  ✗ {f} — absent from the coverage report");
        }
        println!(
            "Cannot certify the coverage floor while changed source files are invisible to it."
        );
        return Ok(1);
    }
    if !report.fn_less_matched.is_empty() {
        bail!(
            "the coverage report matched changed file(s) whose records contain no function data (FN:): {} — function-level coverage cannot be certified for them; use a reporter that emits function data (e.g. cargo llvm-cov --lcov)",
            report.fn_less_matched.join(", ")
        );
    }
    if report.verdict == "pass" && certifies(repo_root, custom_recipe, "cover") {
        hook::write_receipt(repo_root, "cover"); // only a clean, canonical-recipe floor counts
    }

    if report.untested.is_empty() {
        println!("Crucible cover: every changed function is exercised by a test.");
    } else {
        println!(
            "Crucible cover: {} changed function(s) that no test ever calls:",
            report.untested.len()
        );
        for u in &report.untested {
            let tag = if u.high_risk { " [high-risk]" } else { "" };
            println!(
                "  ✗ {}:{} fn {}{tag} — never executed by any test",
                u.file, u.line, u.name
            );
        }
    }
    if !report.untested_branch_lines.is_empty() {
        println!(
            "{} never-taken branch(es) — a code path (else/error arm) no test exercises:",
            report.untested_branch_lines.len()
        );
        for (f, l) in report.untested_branch_lines.iter().take(20) {
            println!("  · {f}:{l}");
        }
    }
    println!(
        "({} uncovered line(s) in changed files.)",
        report.uncovered_line_count
    );
    if report.untested.is_empty() {
        return Ok(0);
    }
    if report.verdict == "block" {
        println!(
            "A never-called function in a high-risk unit is untested behavior shipping to production. Add a test that exercises it."
        );
    } else {
        println!(
            "Advisory here, but a never-called function is not tested at all. Add tests, then `crucible harden` to check the ones that are."
        );
    }
    Ok(if report.verdict == "block" { 1 } else { 0 })
}

// ---- determinism check ----------------------------------------------------

fn cmd_flake(repo_root: &Path, recipe: Option<PathBuf>) -> Result<u8> {
    let recipe_path = recipe.unwrap_or_else(|| repo_root.join(".crucible/flake.json"));
    if !recipe_path.exists() {
        bail!(
            "no flake recipe at {} — create .crucible/flake.json (the test command to run repeatedly)",
            recipe_path.display()
        );
    }
    let recipe: FlakeRecipe = load_json(&recipe_path)?;
    if recipe.cmd.trim().is_empty() || recipe.cmd.starts_with("TODO") {
        bail!("flake.json cmd is not set — point it at the test command to run repeatedly");
    }
    let n = recipe.runs.unwrap_or(3).max(2);
    let cwd = match &recipe.cwd {
        Some(c) => repo_root.join(c),
        None => repo_root.to_path_buf(),
    };
    let timeout = std::time::Duration::from_secs(recipe.timeout_sec.unwrap_or(1800));
    // An invalid failPattern must fail closed: silently dropping it (the old `.ok()`)
    // degraded the check to exit-code-only and still reported "stable" (Codex round 2 #4).
    let fail_re = match &recipe.fail_pattern {
        Some(p) => Some(
            regex::Regex::new(p)
                .with_context(|| format!("flake.json failPattern is not a valid regex: {p}"))?,
        ),
        None => None,
    };

    // Hold one machine-wide slot across all N runs — re-queuing between runs would let
    // another session's build interleave and blow the memory budget.
    let _slot = admission::acquire().map_err(anyhow::Error::msg)?;
    let mut results = vec![];
    for _ in 0..n {
        let out = run_recipe_command(&recipe.cmd, &cwd, timeout, recipe.memory_mb)?;
        // A run killed for memory or runaway output never finished, so its determinism is
        // unproven — fail closed rather than misread it (analyze treats timed_out as
        // inconclusive; both guards map into that).
        if out.memory_exceeded {
            bail!("flake command exceeded its memory limit — the suite did not finish");
        }
        if out.output_exceeded {
            bail!("flake command produced runaway output and was killed to protect the disk");
        }
        results.push(flake::RunResult {
            exit: out.code,
            output: out.output,
            timed_out: out.timed_out,
        });
    }

    let report = flake::analyze(&results, fail_re.as_ref());
    match report.verdict.as_str() {
        "stable" => {
            // "stable" means deterministic, not passing. A suite that fails identically on
            // every run is deterministic but red — via a non-zero exit, or (Codex round 3) a
            // test reported as failing in every run while the process still exits 0. Neither
            // may earn a success receipt or exit 0, which would falsely satisfy the nudge.
            let exit = results[0].exit;
            if exit != 0 {
                println!(
                    "Crucible flake: {n} runs agree (exit {exit}) but the suite failed every run — deterministic, but red. This is not a passing verification; fix the failure."
                );
                return Ok(1);
            }
            if !report.failing_every_run.is_empty() {
                println!(
                    "Crucible flake: {n} runs exit 0, but these test(s) are reported as failing in every run — deterministic, but red, and exit 0 is hiding it:"
                );
                for t in &report.failing_every_run {
                    println!("  ✗ {t}");
                }
                return Ok(1);
            }
            hook::write_receipt(repo_root, "flake"); // only a clean, complete, passing run counts
            println!(
                "Crucible flake: {n} runs produced the same exit code and failure set — no nondeterminism detected."
            );
            return Ok(0);
        }
        "inconclusive" => {
            // A timed-out run never finished, so determinism is unproven. Fail closed.
            println!(
                "Crucible flake: {}/{n} run(s) timed out — the suite did not finish, so determinism could not be verified.",
                report.timed_out_runs
            );
            return Ok(1);
        }
        _ => {}
    }
    println!("Crucible flake: nondeterministic results across {n} runs:");
    if report.exit_inconsistent {
        println!(
            "  ✗ the suite exit code differed between runs (it passed on some, failed on others)"
        );
    }
    for t in &report.flaky_tests {
        println!("  ✗ flaky test: {t} (failed in some runs, passed in others)");
    }
    println!(
        "A flaky green is a false green. Fix the nondeterminism (ordering, timing, shared state) before trusting the suite."
    );
    Ok(1)
}

// ---- adoption / checker ---------------------------------------------------

fn cmd_init(repo_root: &Path, force: bool) -> Result<u8> {
    let r = init::scaffold(repo_root, force)?;
    let display_path = |f: &str| {
        if f.starts_with(".githooks/")
            || f.starts_with("scripts/")
            || f.starts_with("checks/")
            || f.starts_with('/')
        {
            f.to_string()
        } else {
            format!(".crucible/{f}")
        }
    };
    if !r.written.is_empty() {
        println!(
            "Crucible: scaffolded {} file(s) under {}:",
            r.written.len(),
            r.dir.display()
        );
        for f in &r.written {
            println!("  + {}", display_path(f));
        }
    }
    if !r.skipped.is_empty() {
        println!(
            "Kept {} existing file(s) (use --force to overwrite):",
            r.skipped.len()
        );
        for f in &r.skipped {
            println!("  = {}", display_path(f));
        }
    }
    if let Some(note) = &r.hooks_path_note {
        println!("  ✓ {note}");
    }
    if r.written.is_empty() && r.hooks_path_note.is_none() {
        println!("Crucible: already initialized — nothing to write.");
        return Ok(0);
    }
    println!(
        "\nNext steps:
  1. Fill TODOs in .crucible/acceptance.json (build/boot/drive) and
     mutation.json / coverage.json / flake.json for the arms you will use.
  2. Set highRiskUnits in adapter.json (path stems where survivors must block).
  3. Replace checks/check-smoke.sh + the \"smoke\" charter row with real gates,
     or keep them as a placeholder.
  4. Pin (gates first, then config — approving a gate rewrites the charter):
       crucible approve smoke --by <you>
       crucible approve __config__ --by <you>
     Commit approvals separately from the config files.
  5. crucible doctor && crucible check

Docs: docs/GETTING_STARTED.md  ·  docs/ADOPTING.md"
    );
    Ok(0)
}

fn cmd_doctor(repo_root: &Path) -> Result<u8> {
    let checks = doctor::doctor(repo_root);
    for c in &checks {
        let sym = match c.status {
            doctor::Status::Pass => "✓",
            doctor::Status::Warn => "!",
            doctor::Status::Fail => "✗",
        };
        println!("  {sym} {}", c.msg);
    }
    let failed = doctor::any_fail(&checks);
    let warned = checks.iter().any(|c| c.status == doctor::Status::Warn);
    println!();
    if failed {
        if !repo_root.join(".crucible").is_dir() {
            println!("Next:  crucible init");
        } else {
            println!("Next:  fix the ✗ items above, then re-run  crucible doctor");
        }
    } else if warned {
        println!("Healthy with warnings. Address ! items when you can, then run  crucible check");
    } else {
        println!("Healthy. Pin config if you have not:  crucible approve __config__ --by <you>");
    }
    Ok(if failed { 1 } else { 0 })
}

// The plugin wires TUI hook events to `crucible hook <event>`; the payload arrives on
// stdin. Output (a block/context JSON) goes to stdout for the TUI to act on.
fn cmd_hook(event: &str) -> Result<u8> {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let r = hook::run_hook(event, &input);
    if !r.stdout.is_empty() {
        println!("{}", r.stdout);
    }
    Ok(r.exit)
}

fn cmd_test_smells(repo_root: &Path, paths: &[PathBuf], helpers: &[String]) -> Result<u8> {
    if paths.is_empty() {
        bail!("usage: crucible test-smells <file-or-dir> [more...]");
    }
    // A path that does not exist must be an error, not a silent clean pass — otherwise
    // pointing the checker at a typo'd or moved file "passes" while scanning nothing.
    if let Some(missing) = paths.iter().find(|p| !p.exists()) {
        bail!(
            "path does not exist: {} — nothing to scan",
            missing.display()
        );
    }
    let paths: Vec<String> = paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    // Assertion helpers come from the flag plus the repo's .crucible/test-smells.json, so
    // a repo declares its assert helpers once and every run recognizes them. A config
    // that exists but does not parse is an error, not a silently stricter scan.
    let mut all_helpers = helpers.to_vec();
    let cfg_path = repo_root.join(".crucible/test-smells.json");
    if cfg_path.exists() {
        let cfg = load_json::<TestSmellsConfig>(&cfg_path)?;
        all_helpers.extend(cfg.assertion_helpers);
    }
    let scan = smells::inspect_paths(&paths, &all_helpers);
    // Unread input is unverified input: a clean verdict over files the scan could not
    // read, or over nothing at all, is not clean (Codex round 3).
    if !scan.errors.is_empty() {
        eprintln!(
            "test-smells: {} path(s) could not be read — the scan is incomplete:",
            scan.errors.len()
        );
        for e in &scan.errors {
            eprintln!("  ✗ {e}");
        }
        bail!("scan incomplete — fix the unreadable paths and re-run");
    }
    if scan.files_scanned == 0 {
        bail!("no test files found under the given path(s) — nothing was scanned");
    }
    if scan.failures.is_empty() {
        println!(
            "test-smells: {} file(s) scanned, no test-gaming smells found.",
            scan.files_scanned
        );
        Ok(0)
    } else {
        eprintln!(
            "test-smells: {} test-gaming smell(s) in {} file(s) scanned:",
            scan.failures.len(),
            scan.files_scanned
        );
        for f in &scan.failures {
            eprintln!("  ✗ {f}");
        }
        Ok(1)
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
