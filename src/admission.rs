//! Machine-wide admission control for the resource-heavy arms (run/harden/cover/flake).
//!
//! Each of those arms spawns a full build/test process tree (cargo, the app), and a
//! single agent can launch many Crucible sessions at once — one per order, per worktree,
//! per parallel fan-out. Per-session resource caps do NOT compose: N sessions each under
//! a per-tree ceiling can still collectively exhaust system memory and take the machine
//! down. This gate is the piece that composes — it bounds the number of concurrent heavy
//! Crucible trees across ALL sessions on the box to `CRUCIBLE_MAX_CONCURRENCY` (default 1).
//!
//! Slots are OS advisory file locks in the temp dir. The kernel releases a lock when its
//! file handle closes, which happens on normal drop AND on process death, so a crashed or
//! killed session never leaves a stale slot behind. No PID files, no cleanup, no races on
//! reclaim.

use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions, create_dir_all};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const DEFAULT_MAX: u32 = 1;
const DEFAULT_WAIT_SECS: u64 = 3600;

/// Held for the lifetime of a heavy arm. Dropping it (or the process exiting) releases the
/// machine-wide slot.
#[derive(Debug)]
pub struct Slot {
    // The lock lives as long as this handle is open; `_file` is never read, only held.
    _file: File,
}

fn slots_dir() -> PathBuf {
    // An explicit override is honored (a test, or a user who deliberately wants a narrower
    // gate). Otherwise use a FIXED machine-wide path, NOT `std::env::temp_dir()`: temp_dir
    // follows `TMPDIR`, and a fleet that sets a per-session/per-worktree TMPDIR would each
    // get a private slot namespace and defeat the "machine-wide" property (Codex resource
    // review). `/tmp` is the shared location on unix; other platforms fall back to temp_dir.
    if let Ok(dir) = std::env::var("CRUCIBLE_SLOTS_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(unix)]
    {
        PathBuf::from("/tmp/crucible-slots")
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("crucible-slots")
    }
}

/// How many heavy Crucible trees may run at once machine-wide. Default 1 (fully
/// serialized) because the failure mode is OOM-ing the whole box; raise it on a machine
/// with the memory to spare. Capped at the machine's parallelism so an absurd value
/// (e.g. u32::MAX) cannot silently disable the gate — more concurrent heavy builds than
/// cores only multiplies the memory pressure the gate exists to prevent.
pub fn max_concurrency() -> u32 {
    effective_max().0
}

/// Where the effective concurrency came from, so `config` and `doctor` can say which
/// knob is live instead of leaving the user to guess why a setting "didn't take".
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    Env,
    ConfigFile,
    Default,
}

/// The effective slot count and which layer set it: env var > machine config file >
/// default, always capped at core count.
pub fn effective_max() -> (u32, Source) {
    resolve_max(env_max(), file_max(&config_path()), cores())
}

pub fn cores() -> u32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(1)
}

fn env_max() -> Option<u32> {
    std::env::var("CRUCIBLE_MAX_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|n| *n >= 1)
}

// Pure so precedence and the cores cap are unit-tested against fixed inputs instead of
// the machine's real, unassertable core count.
fn resolve_max(env: Option<u32>, file: Option<u32>, cores: u32) -> (u32, Source) {
    let (requested, source) = match (env, file) {
        (Some(n), _) => (n, Source::Env),
        (None, Some(n)) => (n, Source::ConfigFile),
        (None, None) => (DEFAULT_MAX, Source::Default),
    };
    (requested.min(cores.max(1)), source)
}

// ---- machine config file ---------------------------------------------------------

/// Machine config, deliberately NOT repo config: the concurrency budget is a property of
/// the machine, and the adversary in Crucible's threat model edits the repo — a
/// `.crucible/` knob would let an agent grant itself more slots in the tree it is gaming.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct MachineConfig {
    max_concurrency: Option<u32>,
}

/// `$CRUCIBLE_CONFIG_DIR/config.json` (tests, deliberate overrides), else the platform
/// user-config dir. User-owned and outside any repo — see `MachineConfig`.
pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("CRUCIBLE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME")
        && !xdg.trim().is_empty()
    {
        return PathBuf::from(xdg).join("crucible");
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("crucible");
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("crucible")
}

// The file's maxConcurrency, or None if the file is absent or unusable. Unusable warns
// and falls back to the default — the safe direction: a broken config never grants MORE
// concurrency than an intact one.
fn file_max(path: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str::<MachineConfig>(&text) {
        Ok(cfg) => match cfg.max_concurrency {
            Some(0) => {
                eprintln!(
                    "crucible: {} sets maxConcurrency 0 (admits nothing) — using the default",
                    path.display()
                );
                None
            }
            other => other,
        },
        Err(e) => {
            eprintln!(
                "crucible: ignoring malformed {} ({e}) — using the default concurrency",
                path.display()
            );
            None
        }
    }
}

/// Persist `max_concurrency` to the machine config and return the path written.
pub fn set_max(n: u32) -> Result<PathBuf, String> {
    let dir = config_dir();
    create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = dir.join("config.json");
    // Round-trips every MachineConfig field; keys outside the struct are dropped.
    let mut cfg = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<MachineConfig>(&t).ok())
        .unwrap_or_default();
    cfg.max_concurrency = Some(n);
    let json = serde_json::to_string_pretty(&cfg).expect("static struct serializes");
    std::fs::write(&path, json + "\n").map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(path)
}

fn wait_budget() -> Duration {
    let secs = std::env::var("CRUCIBLE_SLOT_WAIT_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_WAIT_SECS);
    Duration::from_secs(secs)
}

/// Block until a machine-wide slot is free, then return a guard whose drop releases it.
/// Waits up to `CRUCIBLE_SLOT_WAIT_SECS` (default one hour) before giving up, so genuinely
/// queued work waits for the machine rather than failing fast.
pub fn acquire() -> Result<Slot, String> {
    acquire_in(&slots_dir(), max_concurrency(), wait_budget())
}

fn acquire_in(dir: &Path, max: u32, timeout: Duration) -> Result<Slot, String> {
    create_dir_all(dir).map_err(|e| format!("creating slot dir {}: {e}", dir.display()))?;
    // Overflow-safe like the proc deadline: an absurd CRUCIBLE_SLOT_WAIT_SECS must not panic.
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(7 * 86_400));
    loop {
        for i in 0..max {
            let path = dir.join(format!("slot-{i}.lock"));
            let file = OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .map_err(|e| format!("opening slot file {}: {e}", path.display()))?;
            if try_lock(&file) {
                return Ok(Slot { _file: file });
            }
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "waited {}s for a free Crucible slot but {max} heavy run(s) are already active machine-wide — raise CRUCIBLE_MAX_CONCURRENCY if this machine has the memory, or let the others finish",
                timeout.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

// A non-blocking exclusive advisory lock. Returns true if this handle now holds it.
#[cfg(unix)]
fn try_lock(file: &File) -> bool {
    use std::os::unix::io::AsRawFd;
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
}

#[cfg(windows)]
fn try_lock(file: &File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        ) != 0
    }
}

#[cfg(not(any(unix, windows)))]
fn try_lock(_file: &File) -> bool {
    // No advisory-lock primitive on this platform: degrade to no gate rather than block
    // forever. The per-tree caps still apply.
    true
}

#[cfg(test)]
#[path = "admission_tests.rs"]
mod tests;
