//! The gate arm: load, validate, and verify an enforcement charter against a repo
//! adapter. Side-effect-free except for reading files under the repo root, so it is
//! unit-testable against fixture repos. Ported 1:1 from the Node reference; the
//! fingerprints are byte-identical so approvals cross-verify between implementations.

use crate::config::{Adapter, Approval, Gate, Ledger};
use crate::hash::{sha256_hex, sha256_hex_of_file};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const TIERS: [&str; 4] = ["T0", "T1", "T2", "T3"];
pub const GATE_TIERS: [&str; 2] = ["T0", "T1"];
pub const BLOCKING_CONDITIONS: [&str; 3] = ["always", "highRisk", "advisory"];
// The adapter's canonical location; it is the trust root, so it must pin itself.
pub const ADAPTER_PATH: &str = ".crucible/adapter.json";

fn resolve(repo_root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(p)
    }
}

// ---- validation -----------------------------------------------------------

pub fn validate_adapter(adapter: &Adapter) -> Vec<String> {
    let mut failures = vec![];
    if adapter.gate_runner.file.is_empty() {
        failures
            .push("adapter.gateRunner.file is required (the gate-runner script to scan)".into());
    }
    if adapter.gate_runner.checker_pattern.is_empty() {
        failures.push(
            "adapter.gateRunner.checkerPattern is required (regex whose group 1 captures a wired checker path)".into(),
        );
    }
    failures
}

pub fn validate_ledger(ledger: &Ledger) -> Vec<String> {
    let mut failures = vec![];
    let mut seen = HashSet::new();
    for (i, g) in ledger.gates.iter().enumerate() {
        let at = if g.id.is_empty() {
            format!("gate[{i}]")
        } else {
            format!("gate[{i}] \"{}\"", g.id)
        };
        if g.id.is_empty() {
            failures.push(format!("{at}: missing \"id\""));
        } else if !seen.insert(g.id.clone()) {
            failures.push(format!("{at}: duplicate id"));
        }
        if g.rule.is_empty() {
            failures.push(format!(
                "{at}: missing \"rule\" (what invariant this enforces)"
            ));
        }
        if !TIERS.contains(&g.tier.as_str()) {
            failures.push(format!("{at}: tier must be one of {}", TIERS.join(", ")));
        }

        let is_gate = GATE_TIERS.contains(&g.tier.as_str()) || g.tier == "T2";
        if is_gate {
            if g.checker.is_none() {
                failures.push(format!("{at}: tier {} requires a \"checker\" path", g.tier));
            }
            if g.oracle_sha256.is_none() {
                failures.push(format!(
                    "{at}: tier {} requires \"oracleSha256\" (the approved digest of the checker)",
                    g.tier
                ));
            }
            if let Some(bc) = &g.blocking_condition
                && !BLOCKING_CONDITIONS.contains(&bc.as_str())
            {
                failures.push(format!(
                    "{at}: blockingCondition must be one of {}",
                    BLOCKING_CONDITIONS.join(", ")
                ));
            }
        }
        // The doctrine's third state must never be silent: a prose or advisory gate
        // carries a written rationale or it is indistinguishable from a rule nobody
        // bothered to enforce.
        let has_reason = g.reason.as_deref().is_some_and(|r| !r.trim().is_empty());
        if g.tier == "T3" && !has_reason {
            failures.push(format!(
                "{at}: T3 (prose-only) requires a non-empty \"reason\" — an unenforced rule must say why it is not a gate"
            ));
        }
        if g.blocking_condition.as_deref() == Some("advisory") && !has_reason {
            failures.push(format!(
                "{at}: blockingCondition \"advisory\" requires a non-empty \"reason\" — why this gate reports but does not block"
            ));
        }
        for (j, tf) in g.trusted_files.iter().enumerate() {
            if tf.path.is_empty() || tf.sha256.is_none() {
                failures.push(format!(
                    "{at}: trustedFiles[{j}] needs both \"path\" and \"sha256\""
                ));
            }
        }
    }
    failures
}

// ---- the honesty check ----------------------------------------------------

// A checker invocation that is commented out does not run, so it must not count as
// wired. `#` or `//` before the match on its line marks it disabled, and a match inside
// a `/* ... */` block is disabled too (Codex round 3: a block-commented invocation
// counted as wired). The runner can be shell, YAML, or JS, so this is a plain scan with
// no string awareness — it fails safe: a false "commented" reading only makes a wired
// gate look un-wired, which fails check rather than passing it.
fn is_commented(text: &str, match_index: usize) -> bool {
    let line_start = text[..match_index].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let before = &text[line_start..match_index];
    if before.contains('#') || before.contains("//") {
        return true;
    }
    in_block_comment(text, match_index)
}

fn in_block_comment(text: &str, match_index: usize) -> bool {
    // Inside a block comment iff an unclosed `/*` opens before the match.
    let before = &text[..match_index];
    match before.rfind("/*") {
        Some(open) => !before[open..].contains("*/"),
        None => false,
    }
}

struct Wired {
    found: HashSet<String>,
    error: Option<String>,
}

fn wired_checkers(repo_root: &Path, adapter: &Adapter) -> Wired {
    let file = resolve(repo_root, &adapter.gate_runner.file);
    if !file.exists() {
        return Wired {
            found: HashSet::new(),
            error: Some(format!(
                "gateRunner.file not found: {}",
                adapter.gate_runner.file
            )),
        };
    }
    let text = match std::fs::read_to_string(&file) {
        Ok(t) => t,
        Err(e) => {
            return Wired {
                found: HashSet::new(),
                error: Some(format!(
                    "reading gateRunner.file {}: {e}",
                    adapter.gate_runner.file
                )),
            };
        }
    };
    let re = match Regex::new(&adapter.gate_runner.checker_pattern) {
        Ok(r) => r,
        Err(e) => {
            return Wired {
                found: HashSet::new(),
                error: Some(format!(
                    "adapter.gateRunner.checkerPattern is not a valid regex: {e}"
                )),
            };
        }
    };
    let mut found = HashSet::new();
    for caps in re.captures_iter(&text) {
        if let Some(g1) = caps.get(1) {
            let idx = caps.get(0).map(|m| m.start()).unwrap_or(g1.start());
            // Comment-out or same-line neutering (`if false`, `|| true`) means the
            // checker is not actually load-bearing.
            if !is_commented(&text, idx) && !crate::pre_push::match_is_neutered(&text, idx) {
                found.insert(g1.as_str().to_string());
            }
        }
    }
    Wired { found, error: None }
}

// The gate's trusted computing base: enforcement metadata (tier, blocking condition,
// checker path, rule) PLUS the checker's bytes PLUS every pinned trustedFile, folded
// into one digest. Including the metadata is what stops an approved gate from being
// silently weakened (dropped from T1 to T2, or switched to advisory) while its old
// approval stays valid.
// Length-prefix every field (`<byte-len>:<value>\n`) so the encoding is injective: a
// delimiter inside a value cannot be read as a field boundary. Without this, a gate with
// checker `c|x` rule `r` and one with checker `c` rule `x|r` fingerprint identically, so
// an approval of one silently validates the other (Codex P1 #9).
fn push_field(buf: &mut String, s: &str) {
    buf.push_str(&s.len().to_string());
    buf.push(':');
    buf.push_str(s);
    buf.push('\n');
}

pub fn gate_fingerprint(repo_root: &Path, gate: &Gate) -> anyhow::Result<String> {
    let checker = gate.checker.as_deref().unwrap_or("");
    let checker_digest = sha256_hex_of_file(&resolve(repo_root, checker))?;
    let mut buf = String::new();
    for part in [
        "gate-v2", // versions the encoding
        gate.tier.as_str(),
        gate.blocking_condition.as_deref().unwrap_or(""),
        checker,
        gate.rule.as_str(),
        checker_digest.as_str(),
    ] {
        push_field(&mut buf, part);
    }
    let mut sorted: Vec<_> = gate.trusted_files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    for tf in sorted {
        let abs = resolve(repo_root, &tf.path);
        let digest = if abs.exists() {
            sha256_hex_of_file(&abs)?
        } else {
            "MISSING".into()
        };
        push_field(&mut buf, &tf.path);
        push_field(&mut buf, &digest);
    }
    Ok(sha256_hex(buf.as_bytes()))
}

// The judge-config trust set is DERIVED, not opt-in (Codex round 3): the adapter, the
// charter, every recipe that exists, and the mutation waivers are always fingerprinted,
// because each one decides what runs, what blocks, and what is exempt. pinnedConfig can
// only add files (e.g. a custom checker config), never subtract — otherwise the trust
// root could opt out of protecting the very files that weaken enforcement (broadened
// waivers, a T1→T3 downgrade, a no-oped recipe). The gate-runner file is deliberately
// not pinned: removing a checker invocation from it is caught by the wiring check, and
// pinning a whole verify script would churn a re-approval on every unrelated edit,
// training reviewers to rubber-stamp.
pub fn judge_config_paths(repo_root: &Path, adapter: &Adapter) -> Vec<String> {
    let mut set: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    set.insert(ADAPTER_PATH.to_string());
    set.insert(adapter.charter.clone());
    for name in [
        "acceptance.json",
        "mutation.json",
        "mutation-waivers.json",
        "coverage.json",
        "flake.json",
        // assertionHelpers decides what counts as an assertion in test-smells, so it is
        // a judge too (Codex round 4).
        "test-smells.json",
    ] {
        let rel = format!(".crucible/{name}");
        if resolve(repo_root, &rel).exists() {
            set.insert(rel);
        }
    }
    // Pre-push is the independence root — its bytes must re-approve when neutered.
    if let Some(pp) = adapter.pre_push.as_deref()
        && resolve(repo_root, pp).exists()
    {
        set.insert(pp.to_string());
    }
    set.extend(adapter.pinned_config.iter().cloned());
    set.into_iter().collect()
}

// Fingerprint of the judge configuration (adapter, recipes, waivers) so those inputs
// cannot be weakened — highRiskUnits emptied, a recipe no-oped, a blanket waiver added
// — without an independent approval.
pub fn config_fingerprint(repo_root: &Path, paths: &[String]) -> anyhow::Result<String> {
    let mut sorted: Vec<&String> = paths.iter().collect();
    sorted.sort();
    let mut buf = String::new();
    push_field(&mut buf, "cfg-v2");
    for p in sorted {
        let abs = resolve(repo_root, p);
        let digest = if abs.exists() {
            sha256_hex_of_file(&abs)?
        } else {
            "MISSING".into()
        };
        push_field(&mut buf, p);
        push_field(&mut buf, &digest);
    }
    Ok(sha256_hex(buf.as_bytes()))
}

// An approval only counts if it names who approved it. Full independence (a second
// human identity) cannot be proven in-core under a single-developer + agents threat
// model — agents commit as the developer. The core requires a non-blank approvedBy,
// verifies pre-push is wired (`pre_push::verify_pre_push`), and flags same-commit
// self-approvals (`pre_push::audit_same_commit_approvals`). See POSITIONING.md.
fn has_approval(approvals: &[Approval], gate_id: &str, fingerprint: &str) -> bool {
    approvals.iter().any(|a| {
        a.gate == gate_id
            && a.approved_by
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty())
            && (a.fingerprint.as_deref() == Some(fingerprint)
                || a.sha256.as_deref() == Some(fingerprint))
    })
}

pub struct CheckResult {
    pub failures: Vec<String>,
    pub warnings: Vec<String>,
}

// Verify a charter is honest: every declared gate resolves to a real checker wired at
// its declared tier, no wired checker is unregistered, and no oracle has drifted
// without an independent approval. `failures` is empty iff the charter passes.
pub fn check_charter(
    repo_root: &Path,
    ledger: &Ledger,
    adapter: &Adapter,
    approvals: &[Approval],
) -> CheckResult {
    let mut failures = validate_adapter(adapter);
    failures.extend(validate_ledger(ledger));
    let mut warnings = vec![];
    if !failures.is_empty() {
        return CheckResult { failures, warnings };
    }

    let wired = wired_checkers(repo_root, adapter);
    if let Some(err) = wired.error {
        failures.push(err);
        return CheckResult { failures, warnings };
    }
    let wired = wired.found;

    let mut registered: HashSet<String> = HashSet::new();

    for g in &ledger.gates {
        let Some(checker) = &g.checker else { continue };
        registered.insert(checker.clone());

        let abs = resolve(repo_root, checker);
        if !abs.exists() {
            failures.push(format!(
                "gate \"{}\": checker file does not exist: {checker}",
                g.id
            ));
            continue;
        }

        // Declared-vs-actual: a T1 gate not wired into the required lane is a lie.
        if g.tier == "T1" && !wired.contains(checker) {
            failures.push(format!(
                "gate \"{}\": declared tier T1 but its checker \"{checker}\" is not wired in the required lane ({})",
                g.id, adapter.gate_runner.file
            ));
        }

        // Oracle integrity: the trusted bytes must be backed by an approval in the log,
        // not merely match the pin the same change declared (self-referential). The
        // approval log is the authority; its independence is enforced at pre-push.
        let mut trusted_missing = false;
        for tf in &g.trusted_files {
            if !resolve(repo_root, &tf.path).exists() {
                failures.push(format!(
                    "gate \"{}\": trustedFile missing: {}",
                    g.id, tf.path
                ));
                trusted_missing = true;
            }
        }
        if trusted_missing {
            continue;
        }

        let fingerprint = match gate_fingerprint(repo_root, g) {
            Ok(fp) => fp,
            Err(e) => {
                failures.push(format!("gate \"{}\": could not fingerprint: {e}", g.id));
                continue;
            }
        };
        if has_approval(approvals, &g.id, &fingerprint) {
            if let Ok(cur) = sha256_hex_of_file(&abs)
                && Some(&cur) != g.oracle_sha256.as_ref()
            {
                warnings.push(format!(
                    "gate \"{}\": ledger oracleSha256 is stale vs the approved checker — re-run `crucible approve {}` to sync the pin",
                    g.id, g.id
                ));
            }
        } else {
            failures.push(format!(
                "gate \"{}\": trusted bytes are not backed by an independent approval — the checker or a trustedFile changed and requires `crucible approve {}` recorded separately from the code change; this is what stops a change from weakening its own gate",
                g.id, g.id
            ));
        }
    }

    // No silent ungated gate: a checker running in the required lane that nobody
    // registered is enforcement the charter cannot account for.
    for c in &wired {
        if !registered.contains(c) {
            failures.push(format!(
                "checker \"{c}\" is wired in {} but is not registered in the charter — every gate must be declared",
                adapter.gate_runner.file
            ));
        }
    }

    // Judge-config integrity: the adapter, charter, recipes, and mutation waivers decide
    // what runs, what blocks, and what is exempt, so they are judges too. The trust set is
    // derived (see judge_config_paths) — the adapter cannot opt out of protecting itself,
    // the charter, or the waivers, so a broadened waiver, an emptied highRiskUnits, or a
    // T1→T3 downgrade always invalidates the __config__ approval. Explicitly pinned files
    // that do not exist are hard failures (a dangling pin is a typo hiding a hole); derived
    // files fingerprint as MISSING when absent, so their appearance or removal also changes
    // the fingerprint.
    let missing: Vec<&String> = adapter
        .pinned_config
        .iter()
        .filter(|p| !resolve(repo_root, p).exists())
        .collect();
    for p in &missing {
        failures.push(format!("pinnedConfig file missing: {p}"));
    }
    if missing.is_empty() {
        let paths = judge_config_paths(repo_root, adapter);
        match config_fingerprint(repo_root, &paths) {
            Ok(fp) => {
                if !has_approval(approvals, "__config__", &fp) {
                    failures.push(format!(
                        "judge config ({}) is not backed by an independent approval — run `crucible approve __config__`; this pins the adapter, charter tiers, highRiskUnits, recipe commands, and mutation waivers so enforcement cannot be weakened silently",
                        paths.join(", ")
                    ));
                }
            }
            Err(e) => failures.push(format!("could not fingerprint judge config: {e}")),
        }
    }

    // Independence layer: pre-push must be a verified fact, not an unreadable adapter field.
    failures.extend(crate::pre_push::verify_pre_push(repo_root, adapter));
    // Audit trail: approvals must not hide inside the same commit as the config they bless.
    failures.extend(crate::pre_push::audit_same_commit_approvals(repo_root, adapter));

    CheckResult { failures, warnings }
}

pub struct AuditReport {
    pub counts: Vec<(String, usize)>,
    pub prose_only: Vec<ProseGate>,
    pub undeclared: Vec<String>,
    pub high_risk_units: Vec<String>,
}

pub struct ProseGate {
    pub id: String,
    pub rule: String,
    pub reason: Option<String>,
}

// Declared-vs-actual delta for the audit: what is gated, what is prose, and what is
// enforced-but-undeclared. Never fails; it reports.
pub fn audit_charter(repo_root: &Path, ledger: &Ledger, adapter: &Adapter) -> AuditReport {
    let wired = wired_checkers(repo_root, adapter).found;
    let mut counts: Vec<(String, usize)> = TIERS.iter().map(|t| (t.to_string(), 0)).collect();
    let mut registered: HashSet<String> = HashSet::new();
    let mut prose_only = vec![];
    for g in &ledger.gates {
        if let Some(slot) = counts.iter_mut().find(|(t, _)| t == &g.tier) {
            slot.1 += 1;
        }
        if let Some(c) = &g.checker {
            registered.insert(c.clone());
        }
        if g.tier == "T3" {
            prose_only.push(ProseGate {
                id: g.id.clone(),
                rule: g.rule.clone(),
                reason: g.reason.clone(),
            });
        }
    }
    let mut undeclared: Vec<String> = wired
        .into_iter()
        .filter(|c| !registered.contains(c))
        .collect();
    undeclared.sort();
    AuditReport {
        counts,
        prose_only,
        undeclared,
        high_risk_units: adapter.high_risk_units.clone(),
    }
}

#[cfg(test)]
#[path = "charter_tests.rs"]
mod tests;
