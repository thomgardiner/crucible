//! `crucible approve`: record an approval of a gate's oracle, or of the judge
//! configuration (`__config__`). Operates on the charter as a JSON value so
//! hand-written keys like `_note` and the original key order survive the rewrite.
//!
//! Commit the approval **separately** from the config/checker it blesses.
//! `crucible check` flags same-commit self-approvals when git history is available
//! and verifies that pre-push actually runs the honesty gate. Cryptographic
//! "approver ≠ author" is out of scope for the single-dev + agents threat model.

use crate::charter::{config_fingerprint, gate_fingerprint, judge_config_paths};
use crate::config::{Adapter, Gate};
use crate::hash::sha256_hex_of_file;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn resolve(repo_root: &Path, p: &str) -> PathBuf {
    let path = Path::new(p);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(p)
    }
}

fn load_value(path: &Path) -> Result<Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn write_pretty(path: &Path, value: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)? + "\n";
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

pub fn approve(
    repo_root: &Path,
    adapter: &Adapter,
    gate_id: &str,
    by: &str,
    note: &str,
) -> Result<String> {
    if by.trim().is_empty() {
        bail!("--by must name the approver — a blank approval carries no accountability");
    }
    let approvals_path = resolve(repo_root, &adapter.approvals);
    let mut approvals = if approvals_path.exists() {
        load_value(&approvals_path)?
    } else {
        json!([])
    };
    let arr = approvals
        .as_array_mut()
        .context("approvals log is not a JSON array")?;
    let at = now_iso8601();

    let fingerprint = if gate_id == "__config__" {
        // The trust set is derived (adapter, charter, recipes, waivers) plus any explicit
        // pinnedConfig extras — the same derivation check_charter verifies against.
        let paths = judge_config_paths(repo_root, adapter);
        // An approval blesses bytes; refuse to bless judge files that do not even parse —
        // a pinned-but-broken recipe would pass check while its arm cannot run (Codex
        // round 7).
        for p in &paths {
            let abs = resolve(repo_root, p);
            if abs.exists() && p.ends_with(".json") {
                load_value(&abs).with_context(|| {
                    format!("refusing to approve judge config: {p} does not parse")
                })?;
            }
        }
        let fp = config_fingerprint(repo_root, &paths)?;
        arr.push(json!({ "gate": "__config__", "fingerprint": fp, "approvedBy": by, "at": at, "note": note }));
        fp
    } else {
        let charter_path = resolve(repo_root, &adapter.charter);
        let mut charter = load_value(&charter_path)?;
        let gates = charter
            .get_mut("gates")
            .and_then(|g| g.as_array_mut())
            .context("charter has no gates array")?;
        let gate_val = gates
            .iter_mut()
            .find(|g| g.get("id").and_then(|i| i.as_str()) == Some(gate_id))
            .with_context(|| format!("no gate \"{gate_id}\" in the ledger"))?;

        let checker = gate_val
            .get("checker")
            .and_then(|c| c.as_str())
            .with_context(|| format!("gate \"{gate_id}\" has no checker to approve"))?
            .to_string();
        let digest = sha256_hex_of_file(&resolve(repo_root, &checker))?;
        gate_val["oracleSha256"] = json!(digest);
        if let Some(tfs) = gate_val
            .get_mut("trustedFiles")
            .and_then(|t| t.as_array_mut())
        {
            for tf in tfs.iter_mut() {
                if let Some(p) = tf.get("path").and_then(|p| p.as_str()) {
                    let d = sha256_hex_of_file(&resolve(repo_root, p))?;
                    tf["sha256"] = json!(d);
                }
            }
        }
        let gate: Gate =
            serde_json::from_value(gate_val.clone()).context("gate does not match the schema")?;
        let fp = gate_fingerprint(repo_root, &gate)?;
        write_pretty(&charter_path, &charter)?;
        arr.push(json!({
            "gate": gate_id, "fingerprint": fp, "checkerSha256": digest,
            "approvedBy": by, "at": at, "note": note,
        }));
        fp
    };

    write_pretty(&approvals_path, &approvals)?;
    Ok(fingerprint)
}

// RFC3339 UTC, matching the Node `new Date().toISOString()` shape. The approval's `at`
// is informational (the check verifies approvedBy + fingerprint), but a real timestamp
// keeps the log auditable without pulling in a date dependency.
fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86400, secs % 86400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

// Howard Hinnant's days-from-civil, inverted: days since 1970-01-01 → (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
#[path = "approve_tests.rs"]
mod tests;
