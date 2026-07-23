//! Linux cgroup-v2 containment for spawned build/test trees, via `systemd-run --user
//! --scope`. Where available it gives KERNEL-enforced limits the polling fallback cannot:
//!
//! - `MemoryMax` OOM-kills the scope the instant it exceeds — no 20 ms poll window a fast
//!   allocator could slip through.
//! - `MemorySwapMax=0` stops the limit being dodged by swapping.
//! - `TasksMax` caps the process/thread count, stopping a fork bomb.
//! - Killing the scope's cgroup reaches EVERY process in it, including a `setsid` daemon
//!   that reparented out of the process group, and the scope is torn down with the run.
//!
//! It is unavailable on non-Linux hosts, without a user systemd manager, or without cgroup
//! delegation — there the caller falls back to the process-group containment in `proc.rs`,
//! so behaviour on those hosts is unchanged. Set `CRUCIBLE_NO_CGROUP=1` to force the
//! fallback (e.g. to compare, or on a host where the scope misbehaves).
//!
//! The kernel-enforcement path can only be verified on a Linux host with a user systemd
//! session; the pure parts (availability decision, argv construction, unit naming) are unit
//! tested everywhere.

use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

// A generous per-tree task ceiling: a big `cargo -j` build spawns many rustc processes and
// threads (low hundreds), so this never bites a real build, but a fork bomb hits it fast.
const DEFAULT_TASKS_MAX: u64 = 8192;

static AVAILABLE: OnceLock<bool> = OnceLock::new();
static SEQ: AtomicU64 = AtomicU64::new(0);

/// True when `systemd-run --user --scope` can create a limited cgroup on this host.
/// Cached: the probe runs once per process.
pub fn available() -> bool {
    *AVAILABLE.get_or_init(detect)
}

#[cfg(target_os = "linux")]
fn detect() -> bool {
    if std::env::var_os("CRUCIBLE_NO_CGROUP").is_some() {
        return false;
    }
    // A running user systemd manager exposes this directory; without it `--user` fails.
    let has_user_manager = std::env::var_os("XDG_RUNTIME_DIR")
        .map(|d| std::path::Path::new(&d).join("systemd/private").exists())
        .unwrap_or(false);
    if !has_user_manager {
        return false;
    }
    // Probe for real: create a throwaway scope that runs `true`. If this succeeds, delegation
    // and the transient-scope path work; if anything is missing, fall back.
    Command::new("systemd-run")
        .args([
            "--user",
            "--scope",
            "--quiet",
            "--collect",
            "--property=TasksMax=16",
            "--",
            "true",
        ])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
fn detect() -> bool {
    false
}

/// A unique transient scope-unit name for one spawned tree. Unique per (process, spawn) so
/// concurrent or sequential runs never collide.
pub fn unit_name() -> String {
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("crucible-{}-{seq}.scope", std::process::id())
}

/// Build the argv that runs `inner` (the shell invocation, e.g. `["sh","-c",cmd]`) inside a
/// limited transient scope named `unit`. `memory_bytes` is the kernel `MemoryMax`; None
/// leaves memory unbounded by the cgroup (the polling ceiling still applies).
pub fn wrap(unit: &str, memory_bytes: Option<u64>, inner: &[String]) -> Vec<String> {
    let mut argv = vec![
        "systemd-run".to_string(),
        "--user".to_string(),
        "--scope".to_string(),
        "--quiet".to_string(),
        // --collect garbage-collects the scope even if the payload fails, so a failed run
        // never leaves a lingering unit.
        "--collect".to_string(),
        format!("--unit={unit}"),
        format!("--property=TasksMax={DEFAULT_TASKS_MAX}"),
        "--property=MemorySwapMax=0".to_string(),
    ];
    if let Some(bytes) = memory_bytes {
        argv.push(format!("--property=MemoryMax={bytes}"));
    }
    argv.push("--".to_string());
    argv.extend(inner.iter().cloned());
    argv
}

/// Kill every process in the scope's cgroup, reaching even a `setsid` daemon that left the
/// process group. Best-effort: a missing/already-gone unit is not an error.
pub fn kill(unit: &str) {
    let _ = Command::new("systemctl")
        .args([
            "--user",
            "kill",
            "--kill-whom=all",
            "--signal=SIGKILL",
            unit,
        ])
        .output();
}

#[cfg(test)]
#[path = "cgroup_tests.rs"]
mod tests;
