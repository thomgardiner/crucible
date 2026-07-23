//! `crucible doctor`: adoption health. Reports whether a repo's `.crucible/` config is
//! present, parses, resolves its gate runner, has its mutation tool on PATH, and passes
//! the honesty check. A fast "is this wired right?" pass before the real gates run.
//! Any Fail exits non-zero.

use crate::charter::check_charter;
use crate::config::{Adapter, Approval, Ledger, load_json};
use std::path::Path;

#[derive(PartialEq)]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

pub struct Check {
    pub status: Status,
    pub msg: String,
}

fn pass(msg: impl Into<String>) -> Check {
    Check {
        status: Status::Pass,
        msg: msg.into(),
    }
}
fn warn(msg: impl Into<String>) -> Check {
    Check {
        status: Status::Warn,
        msg: msg.into(),
    }
}
fn fail(msg: impl Into<String>) -> Check {
    Check {
        status: Status::Fail,
        msg: msg.into(),
    }
}

pub fn doctor(repo_root: &Path) -> Vec<Check> {
    let mut checks = vec![];

    checks.push(if on_path("git") {
        pass("git is available")
    } else {
        fail("git not found on PATH — the mutation gate's diff scoping needs it")
    });

    // Which resource-containment path the heavy arms will use on this host.
    checks.push(if crate::cgroup::available() {
        pass("resource containment: kernel-enforced (systemd-run cgroup scope)")
    } else {
        warn(
            "resource containment: process-group fallback (no user cgroup scope) — a killed \
             or setsid'd tree can escape; see the README containment table",
        )
    });

    let (slots, source) = crate::admission::effective_max();
    let source = match source {
        crate::admission::Source::Env => "env CRUCIBLE_MAX_CONCURRENCY",
        crate::admission::Source::ConfigFile => "machine config",
        crate::admission::Source::Default => "default",
    };
    checks.push(pass(format!(
        "concurrency gate: {slots} heavy run(s) at once machine-wide ({source}) — change \
         with `crucible config max-concurrency <N>`"
    )));

    let cru = repo_root.join(".crucible");
    if !cru.exists() {
        checks.push(fail(
            ".crucible/ not found — run `crucible init` to scaffold it",
        ));
        return checks;
    }

    let adapter = match load_json::<Adapter>(&cru.join("adapter.json")) {
        Ok(a) => {
            checks.push(pass(".crucible/adapter.json parses"));
            a
        }
        Err(e) => {
            checks.push(fail(format!(".crucible/adapter.json does not parse: {e}")));
            return checks;
        }
    };

    let ledger = match load_json::<Ledger>(&repo_root.join(&adapter.charter)) {
        Ok(l) => {
            checks.push(pass(format!(
                "{} parses ({} gates)",
                adapter.charter,
                l.gates.len()
            )));
            l
        }
        Err(e) => {
            checks.push(fail(format!("{} does not parse: {e}", adapter.charter)));
            return checks;
        }
    };

    if adapter.gate_runner.file.is_empty() || !repo_root.join(&adapter.gate_runner.file).exists() {
        checks.push(fail(format!(
            "gate runner not found: {} — fill adapter.gateRunner.file",
            adapter.gate_runner.file
        )));
    } else {
        checks.push(pass(format!(
            "gate runner resolves ({})",
            adapter.gate_runner.file
        )));
    }

    // Pre-push is load-bearing for the independence claim — not a decorative adapter field.
    let pre_push_fails = crate::pre_push::verify_pre_push(repo_root, &adapter);
    if pre_push_fails.is_empty() {
        checks.push(pass(format!(
            "pre-push hook wires crucible check ({})",
            adapter.pre_push.as_deref().unwrap_or("?")
        )));
    } else {
        for f in pre_push_fails {
            checks.push(fail(f));
        }
    }
    if let Some(msg) = crate::pre_push::hooks_path_status(repo_root, &adapter) {
        checks.push(warn(msg));
    }

    // Every recipe-driven arm gets the same wiring check: a TODO command or a missing
    // tool is caught here, before the arm itself fails at verification time.
    for (file, arm) in [
        ("mutation.json", "harden"),
        ("coverage.json", "cover"),
        ("flake.json", "flake"),
    ] {
        let path = cru.join(file);
        if !path.exists() {
            continue;
        }
        // A recipe that exists but cannot run its arm is an unhealthy adoption, not a
        // silent skip (Codex round 7): byte-level config approval would still pass it.
        let value = match load_json::<serde_json::Value>(&path) {
            Ok(v) => v,
            Err(e) => {
                checks.push(fail(format!("{file} does not parse: {e}")));
                continue;
            }
        };
        let cmd = value.get("cmd").and_then(|c| c.as_str()).map(String::from);
        let Some(bin) = cmd.as_deref().and_then(|c| c.split_whitespace().next()) else {
            checks.push(fail(format!(
                "{file} has no \"cmd\" — `crucible {arm}` cannot run"
            )));
            continue;
        };
        if bin.starts_with("TODO") {
            checks.push(warn(format!(
                "{file} cmd is still a TODO — fill it before `crucible {arm}`"
            )));
        } else if on_path(bin) {
            checks.push(pass(format!("{arm} tool '{bin}' is on PATH")));
        } else {
            checks.push(warn(format!(
                "{arm} tool '{bin}' not on PATH — `crucible {arm}` will fail"
            )));
        }
    }

    let approvals: Vec<Approval> =
        load_json(&repo_root.join(&adapter.approvals)).unwrap_or_default();
    let result = check_charter(repo_root, &ledger, &adapter, &approvals);
    if result.failures.is_empty() {
        checks.push(pass(format!(
            "charter is honest ({} gates, 0 violations)",
            ledger.gates.len()
        )));
    } else {
        for f in result.failures {
            checks.push(fail(f));
        }
    }
    checks
}

pub fn any_fail(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.status == Status::Fail)
}

// Whether an executable named `bin` is resolvable on PATH, without spawning it.
fn on_path(bin: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        if dir.join(bin).is_file() {
            return true;
        }
        if cfg!(windows) {
            for ext in ["exe", "cmd", "bat"] {
                if dir.join(format!("{bin}.{ext}")).is_file() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "doctor_tests.rs"]
mod tests;
