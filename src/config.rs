//! Typed views of the `.crucible/` JSON config. Field names are snake_case with a
//! camelCase serde rename so the on-disk keys (gateRunner, highRiskUnits,
//! oracleSha256, …) match the Node reference and the JSON Schemas unchanged. Unknown
//! keys (e.g. a recipe's `_note`) are ignored on read, so hand-annotated config loads.

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn load_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

// ---- gate arm -------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Adapter {
    #[allow(dead_code)] // schema field, not yet consumed
    pub repo: Option<String>,
    #[serde(default = "default_charter")]
    pub charter: String,
    #[serde(default = "default_approvals")]
    pub approvals: String,
    #[serde(default)]
    pub gate_runner: GateRunner,
    #[allow(dead_code)] // schema field, not yet consumed
    pub change_to_units: Option<String>,
    #[serde(default)]
    pub high_risk_units: Vec<String>,
    /// Path to the pre-push hook. Load-bearing: `check`/`doctor` require it to exist
    /// and run `crucible check` (see `pre_push` module). Not cryptographic independence.
    pub pre_push: Option<String>,
    #[serde(default)]
    pub pinned_config: Vec<String>,
}

fn default_charter() -> String {
    ".crucible/charter.json".into()
}
fn default_approvals() -> String {
    ".crucible/approvals.json".into()
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GateRunner {
    #[allow(dead_code)] // schema field: human-facing description of the gate command
    pub command: Option<String>,
    #[serde(default)]
    pub file: String,
    #[serde(default)]
    pub checker_pattern: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ledger {
    #[serde(default)]
    pub gates: Vec<Gate>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Gate {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub rule: String,
    #[serde(default)]
    pub tier: String,
    pub checker: Option<String>,
    pub oracle_sha256: Option<String>,
    #[serde(default)]
    pub trusted_files: Vec<TrustedFile>,
    pub blocking_condition: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TrustedFile {
    pub path: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub gate: String,
    pub fingerprint: Option<String>,
    // Legacy alias for fingerprint, accepted so old approval logs keep verifying.
    pub sha256: Option<String>,
    pub approved_by: Option<String>,
}

// ---- reality arm ----------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub repo: Option<String>,
    pub build: Option<Step>,
    pub boot: Option<Step>,
    #[serde(default)]
    pub drive: Vec<Step>,
    pub trust: Option<TrustCfg>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub name: Option<String>,
    #[serde(default)]
    pub cmd: String,
    pub cwd: Option<String>,
    pub timeout_sec: Option<u64>,
    pub oracle: Option<Oracle>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Oracle {
    pub stdout_match: Option<String>,
    pub stdout_forbid: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustCfg {
    #[serde(default)]
    pub test_roots: Vec<String>,
    #[serde(default)]
    pub test_pattern: String,
    #[serde(default)]
    pub mock_markers: Vec<String>,
}

// ---- mutation arm ---------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutationRecipe {
    #[serde(default)]
    pub cmd: String,
    pub base: Option<String>,
    pub cwd: Option<String>,
    pub timeout_sec: Option<u64>,
    pub memory_mb: Option<u64>,
    pub survivor_pattern: Option<String>,
    // Proof the mutation run actually happened. A tool that emits no summary (e.g. `true`)
    // must not read as "no survivors, all clean" — the harden gate requires the output to
    // match this before trusting a zero-survivor result. Defaults to cargo-mutants' summary.
    pub completion_pattern: Option<String>,
}

// Optional per-repo config for the test-smells checker.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestSmellsConfig {
    #[serde(default)]
    pub assertion_helpers: Vec<String>,
}

// The determinism recipe: a test command run N times to detect nondeterminism.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlakeRecipe {
    #[serde(default)]
    pub cmd: String,
    pub runs: Option<u32>,
    // Regex whose group 1 is a failed test's name; without it, only exit codes are compared.
    pub fail_pattern: Option<String>,
    pub cwd: Option<String>,
    pub timeout_sec: Option<u64>,
    // Optional process-tree memory ceiling in MiB. Opt-in (unlike harden's default) because
    // a full test suite legitimately uses a lot; set it to cap a runaway.
    pub memory_mb: Option<u64>,
}

// The coverage floor recipe: a command that emits LCOV (to lcovPath, or stdout).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageRecipe {
    #[serde(default)]
    pub cmd: String,
    pub base: Option<String>,
    pub cwd: Option<String>,
    pub timeout_sec: Option<u64>,
    pub lcov_path: Option<String>,
    // Optional process-tree memory ceiling in MiB. Opt-in because a coverage build
    // (e.g. cargo llvm-cov) legitimately uses a lot; set it to cap a runaway.
    pub memory_mb: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Waiver {
    pub file: Option<String>,
    pub line: Option<u64>,
    pub mutation: Option<String>,
    pub reason: Option<String>,
}
