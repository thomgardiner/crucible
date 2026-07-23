//! Run a shell command under a hard timeout, capturing combined stdout+stderr, and
//! kill the whole process tree if it overruns. A recipe's real work (cargo, the app)
//! runs in grandchild processes, so a bare child-kill would leave them alive holding
//! the pipes open — the group/job kill is mirrored from Summoner's `backend_provenance`.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::collections::HashSet;

// How many bytes of the capture file we READ back (the tail shown to the user).
const CAP: u64 = 500_000;
// Hard ceiling on how many bytes a spawned command may WRITE to its capture file before
// the whole process tree is killed. A runaway `yes`-style command hits this in well under
// a second, so it protects the temp partition from being filled and the machine taken
// down, while sitting far above any real build/test log. Unconditional on every arm.
const OUTPUT_CAP: u64 = 256 * 1024 * 1024;

// Whether the captured output (stdout + stderr) has crossed the disk-protection ceiling.
// Pure and saturating so the boundary is unit-tested without writing OUTPUT_CAP bytes, and
// so a summed length near u64::MAX cannot wrap the comparison.
fn over_output_cap(out_len: u64, err_len: u64) -> bool {
    out_len.saturating_add(err_len) > OUTPUT_CAP
}

#[derive(Debug)]
pub struct Output {
    pub code: i32,
    pub output: String,
    pub timed_out: bool,
    pub memory_exceeded: bool,
    // The command wrote more than OUTPUT_CAP bytes and was killed to protect the disk.
    pub output_exceeded: bool,
}

/// The process boundary, injected so `run`/`harden` cores are testable without
/// spawning (mock only the system edge, never the internal logic).
pub trait Exec {
    fn run(&self, cmd: &str, cwd: &Path, timeout: Duration) -> Output;

    fn run_limited(&self, cmd: &str, cwd: &Path, timeout: Duration, memory_bytes: u64) -> Output {
        let _ = memory_bytes;
        self.run(cmd, cwd, timeout)
    }
}

pub struct ShellExec;

impl Exec for ShellExec {
    fn run(&self, cmd: &str, cwd: &Path, timeout: Duration) -> Output {
        run_shell(cmd, cwd, timeout, None)
    }

    fn run_limited(&self, cmd: &str, cwd: &Path, timeout: Duration, memory_bytes: u64) -> Output {
        run_shell(cmd, cwd, timeout, Some(memory_bytes))
    }
}

// The shell invocation as an argv, used both directly and as the payload inside a cgroup
// scope on Linux.
fn shell_argv(cmd: &str) -> Vec<String> {
    if cfg!(windows) {
        vec!["cmd".into(), "/C".into(), cmd.into()]
    } else {
        vec!["sh".into(), "-c".into(), cmd.into()]
    }
}

// Build the Command to spawn, plus the cgroup scope unit it runs inside (when kernel
// containment is available on this host). On Linux with `systemd-run --user --scope`, the
// tree runs in a scope whose `MemoryMax`/`TasksMax` the kernel enforces; everywhere else
// this is the plain shell command and the process-group fallback applies.
fn build_command(cmd: &str, memory_limit: Option<u64>) -> (Command, Option<String>) {
    let inner = shell_argv(cmd);
    if crate::cgroup::available() {
        let unit = crate::cgroup::unit_name();
        let argv = crate::cgroup::wrap(&unit, memory_limit, &inner);
        let mut c = Command::new(&argv[0]);
        c.args(&argv[1..]);
        (c, Some(unit))
    } else {
        let mut c = Command::new(&inner[0]);
        c.args(&inner[1..]);
        (c, None)
    }
}

/// MiB → bytes for a process-tree memory ceiling, rejecting a non-positive value (a
/// `memoryMb` of 0 would kill every command instantly). Shared by every arm that caps
/// memory so the validation is identical.
pub fn memory_limit_bytes(memory_mb: u64) -> Result<u64, String> {
    memory_mb
        .checked_mul(1024 * 1024)
        .filter(|bytes| *bytes > 0)
        .ok_or_else(|| "memoryMb must be a positive MiB value".into())
}

/// Total physical RAM in bytes, or None when it cannot be read.
#[cfg(target_os = "linux")]
pub fn total_ram_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemTotal:"))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kb.saturating_mul(1024))
}

#[cfg(target_os = "macos")]
pub fn total_ram_bytes() -> Option<u64> {
    let out = Command::new("/usr/sbin/sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(windows)]
pub fn total_ram_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 {
        Some(status.ullTotalPhys)
    } else {
        None
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn total_ram_bytes() -> Option<u64> {
    None
}

/// The default per-tree memory ceiling in bytes so heavy work is always bounded even when
/// a recipe sets no `memoryMb`. It composes with the concurrency gate: the whole safety
/// budget (a fraction of RAM) is divided by how many heavy trees may run at once, so N
/// trees together never exceed that fraction. Returns None only when RAM cannot be read
/// (then the caller keeps the arm's own explicit limit or runs uncapped, and says so).
pub fn default_memory_bytes(max_concurrency: u32) -> Option<u64> {
    Some(budget_bytes(total_ram_bytes()?, max_concurrency))
}

// The per-tree share of the heavy-work memory budget: 80% of total RAM (above that the
// machine is already thrashing), split across the allowed concurrency so N same-configured
// trees together stay under that 80%. Pure so the ratio is unit-tested against known inputs
// instead of the machine's real, unassertable RAM.
fn budget_bytes(total: u64, max_concurrency: u32) -> u64 {
    const SAFE_NUMERATOR: u64 = 4; // 4/5 = 80%
    const SAFE_DENOMINATOR: u64 = 5;
    let budget = total / SAFE_DENOMINATOR * SAFE_NUMERATOR;
    budget / max_concurrency.max(1) as u64
}

// ---- kill-on-signal cleanup ------------------------------------------------
//
// If crucible itself is killed or interrupted (SIGTERM/SIGINT/SIGHUP — the "kill the
// agent" case), its spawned build tree would otherwise survive as an orphan still eating
// RAM, and because the dead session releases its admission slot, new sessions pile on top
// of it. A lock-free registry of active process-group ids lets a signal handler reap them
// all before crucible exits. (SIGKILL is uncatchable and still leaks — that needs a
// cgroup/Job watchdog; Windows already reaps via KILL_ON_JOB_CLOSE.)

#[cfg(unix)]
use std::sync::atomic::{AtomicI32, Ordering};

#[cfg(unix)]
const MAX_TRACKED_GROUPS: usize = 1024;

#[cfg(unix)]
static ACTIVE_GROUPS: [AtomicI32; MAX_TRACKED_GROUPS] =
    [const { AtomicI32::new(0) }; MAX_TRACKED_GROUPS];

// Register a process-group id for reap-on-signal; returns its slot, or None if the (large)
// table is somehow full — the caller then relies on the normal in-loop terminate paths.
#[cfg(unix)]
fn track_group(pgid: i32) -> Option<usize> {
    ACTIVE_GROUPS.iter().position(|slot| {
        slot.compare_exchange(0, pgid, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    })
}

#[cfg(unix)]
fn untrack_group(slot: usize) {
    ACTIVE_GROUPS[slot].store(0, Ordering::Release);
}

// Async-signal-safe: atomic loads and `kill` are safe to call from a handler. Reap every
// tracked group, then restore the default disposition and re-raise so crucible exits with
// the signal's own semantics.
#[cfg(unix)]
extern "C" fn reap_on_signal(sig: i32) {
    for slot in ACTIVE_GROUPS.iter() {
        let pgid = slot.load(Ordering::Acquire);
        if pgid != 0 {
            unsafe {
                libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

/// Install the kill-on-signal cleanup. Call once at startup so an interrupted or terminated
/// crucible does not leave its build tree running.
#[cfg(unix)]
pub fn install_signal_cleanup() {
    let handler = reap_on_signal as *const () as libc::sighandler_t;
    unsafe {
        libc::signal(libc::SIGTERM, handler);
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGHUP, handler);
    }
}

#[cfg(not(unix))]
pub fn install_signal_cleanup() {}

// An early-exit Output for a setup failure, before any child runs.
fn setup_error(code: i32, output: String) -> Output {
    Output {
        code,
        output,
        timed_out: false,
        memory_exceeded: false,
        output_exceeded: false,
    }
}

fn run_shell(cmd: &str, cwd: &Path, timeout: Duration, memory_limit: Option<u64>) -> Output {
    let file = match tempfile::tempfile() {
        Ok(f) => f,
        Err(e) => return setup_error(127, format!("crucible: could not open capture file: {e}")),
    };
    let (out_handle, err_handle) = match (file.try_clone(), file.try_clone()) {
        (Ok(a), Ok(b)) => (a, b),
        _ => return setup_error(127, "crucible: could not clone capture file".into()),
    };

    let (mut command, scope_unit) = build_command(cmd, memory_limit);
    command
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_handle))
        .stderr(Stdio::from(err_handle));
    configure_tree(&mut command);

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => return setup_error(127, format!("crucible: spawning `{cmd}` failed: {e}")),
    };

    let tree = match ProcessTree::attach(&child, memory_limit, scope_unit) {
        Ok(tree) => tree,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Output {
                code: 125,
                output: format!("crucible: could not enforce memory limit: {error}"),
                timed_out: false,
                memory_exceeded: true,
                output_exceeded: false,
            };
        }
    };
    // Overflow-safe: an absurd `timeoutSec` (e.g. u64::MAX) must not panic on the add and
    // orphan the child. checked_add falls back to a far-future-but-valid deadline.
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(7 * 86_400));
    let mut timed_out = false;
    let mut memory_exceeded = false;
    let mut output_exceeded = false;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // The leader exited. A recipe that backgrounds work (`cmd &`) leaves members
                // alive in our process group; terminate() kills them but only when a live
                // member still holds the pgid, so it can never signal a recycled id.
                tree.terminate();
                break exit_code(status.code());
            }
            Ok(None) => {}
            // A try_wait error leaves the child in an unknown state; kill the group and reap
            // rather than orphaning it.
            Err(_) => {
                tree.terminate();
                let _ = child.wait();
                break 1;
            }
        }
        if Instant::now() >= deadline {
            tree.terminate();
            let _ = child.wait();
            timed_out = true;
            break 124;
        }
        // Disk guard: a command writing without bound would fill the temp partition. Kill
        // the whole tree the moment its capture file crosses the ceiling.
        if over_output_cap(file.metadata().map(|m| m.len()).unwrap_or(0), 0) {
            tree.terminate();
            let _ = child.wait();
            output_exceeded = true;
            break 126;
        }
        if let Some(limit) = memory_limit {
            match tree.memory_bytes() {
                Ok(bytes) if bytes > limit => {
                    tree.terminate();
                    let _ = child.wait();
                    memory_exceeded = true;
                    break 125;
                }
                Ok(_) => {}
                Err(error) => {
                    tree.terminate();
                    let _ = child.wait();
                    return Output {
                        code: 125,
                        output: format!("crucible: could not enforce memory limit: {error}"),
                        timed_out: false,
                        memory_exceeded: true,
                        output_exceeded: false,
                    };
                }
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Output {
        code: if timed_out { 124 } else { code },
        output: tail_file(file),
        timed_out,
        memory_exceeded,
        output_exceeded,
    }
}

/// The result of a bounded external-program run: full stdout/stderr (each capped) and
/// whether the deadline killed it.
#[derive(Debug)]
pub struct ProgramOutput {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

/// Run an external program under a hard timeout with a whole-process-group kill, for the
/// pre-slot discovery work `harden`/`cover` do — the `git diff`/`git ls-files` that scope
/// the gate to the changed files, BEFORE the machine-wide slot is acquired. That git is
/// PATH-resolved; unwrapped, a hung or hostile one (a filesystem lock, a credential
/// prompt, an infinite hook) would hang Crucible outside every resource cap. This applies
/// the same deadline + group kill as a recipe command, without the cgroup/memory-poll
/// machinery a build tree needs. stdout and stderr go to separate files (parse one, report
/// the other) and each is bounded so a flooding program cannot fill the temp partition.
pub fn run_program_bounded(
    program: &str,
    args: &[&str],
    cwd: &Path,
    timeout: Duration,
) -> std::io::Result<ProgramOutput> {
    let out_file = tempfile::tempfile()?;
    let err_file = tempfile::tempfile()?;
    let (out_handle, err_handle) = (out_file.try_clone()?, err_file.try_clone()?);

    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_handle))
        .stderr(Stdio::from(err_handle));
    configure_tree(&mut command);

    let mut child = command.spawn()?;
    // No memory cap and no cgroup scope: discovery is light and runs before the slot gate,
    // so a per-call scope spin-up would cost more than it saves. The process-group kill is
    // the containment that matters — a hang cannot outlive the deadline.
    let tree = match ProcessTree::attach(&child, None, None) {
        Ok(t) => t,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::other(e.to_string()));
        }
    };
    let deadline = Instant::now()
        .checked_add(timeout)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(7 * 86_400));
    let mut timed_out = false;
    let captured = |f: &std::fs::File| f.metadata().map(|m| m.len()).unwrap_or(0);
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                tree.terminate();
                break exit_code(status.code());
            }
            Ok(None) => {}
            Err(_) => {
                tree.terminate();
                let _ = child.wait();
                break 1;
            }
        }
        if Instant::now() >= deadline {
            tree.terminate();
            let _ = child.wait();
            timed_out = true;
            break 124;
        }
        // Even discovery is bounded: a program flooding stdout must not fill the disk
        // before the deadline. Non-zero exit makes the caller treat the run as failed.
        if over_output_cap(captured(&out_file), captured(&err_file)) {
            tree.terminate();
            let _ = child.wait();
            break 126;
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    Ok(ProgramOutput {
        code: if timed_out { 124 } else { code },
        stdout: read_capped(out_file),
        stderr: read_capped(err_file),
        timed_out,
    })
}

// Read a capture file from the start, up to the output ceiling. Unlike tail_file this
// keeps the head — a filename list is parsed whole — but still never loads an unbounded
// runaway into memory.
fn read_capped(mut file: std::fs::File) -> String {
    let _ = file.seek(SeekFrom::Start(0));
    let mut bytes = Vec::new();
    let _ = file.take(OUTPUT_CAP).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

fn exit_code(code: Option<i32>) -> i32 {
    code.unwrap_or(1)
}

// Read the last CAP bytes of the capture file: a runaway command's multi-gigabyte
// log must not be loaded whole.
fn tail_file(mut file: std::fs::File) -> String {
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(len.saturating_sub(CAP)));
    let mut bytes = Vec::new();
    let _ = file.take(CAP).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(unix)]
fn configure_tree(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(not(unix))]
fn configure_tree(_command: &mut Command) {}

struct ProcessTree {
    #[cfg(unix)]
    pid: u32,
    #[cfg(unix)]
    track_slot: Option<usize>,
    #[cfg(unix)]
    terminated: std::cell::Cell<bool>,
    // The cgroup scope this tree runs in, when kernel containment is active (Linux). Killing
    // the scope reaches every process in it, including a setsid escape.
    #[cfg(unix)]
    scope_unit: Option<String>,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(not(any(unix, windows)))]
    _priv: (),
}

impl ProcessTree {
    #[cfg(unix)]
    fn attach(
        child: &std::process::Child,
        _memory_limit: Option<u64>,
        scope_unit: Option<String>,
    ) -> Result<Self, String> {
        let pid = child.id();
        // Register for reap-on-signal: if crucible is killed mid-run, the handler kills
        // this group (pgid == pid, the child is its group leader) instead of orphaning it.
        let track_slot = track_group(pid as i32);
        Ok(Self {
            pid,
            track_slot,
            terminated: std::cell::Cell::new(false),
            scope_unit,
        })
    }

    #[cfg(windows)]
    fn attach(
        child: &std::process::Child,
        memory_limit: Option<u64>,
        _scope_unit: Option<String>,
    ) -> Result<Self, String> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_JOB_MEMORY,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject,
        };
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() || job == INVALID_HANDLE_VALUE {
                return Err("CreateJobObjectW failed".into());
            }
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(bytes) = memory_limit {
                limits.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_JOB_MEMORY;
                limits.JobMemoryLimit = bytes as usize;
            }
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            ) == 0
            {
                CloseHandle(job);
                return Err("SetInformationJobObject failed".into());
            }
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, child.id());
            if process.is_null() || process == INVALID_HANDLE_VALUE {
                CloseHandle(job);
                return Err("OpenProcess failed".into());
            }
            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                CloseHandle(job);
                return Err("AssignProcessToJobObject failed".into());
            }
            Ok(Self { job })
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn attach(
        _child: &std::process::Child,
        memory_limit: Option<u64>,
        _scope_unit: Option<String>,
    ) -> Result<Self, String> {
        if memory_limit.is_some() {
            Err("memory limits are unsupported on this platform".into())
        } else {
            Ok(Self { _priv: () })
        }
    }

    #[cfg(unix)]
    fn memory_bytes(&self) -> Result<u64, String> {
        // Sum RSS by process GROUP, not parent chain. The child is a group leader
        // (process_group(0) → pgid == its pid == self.pid), so this counts every member of
        // the group — including a worker whose intermediate parent exited and reparented to
        // init but stayed in the group, which a PPID walk would miss (Codex resource
        // review). This matches exactly what `kill(-pgid)` will reap.
        let kib = unix_processes()?
            .iter()
            .filter(|process| process.pgid == self.pid)
            .map(|process| process.rss_kib)
            .sum::<u64>();
        Ok(kib.saturating_mul(1024))
    }

    #[cfg(windows)]
    fn memory_bytes(&self) -> Result<u64, String> {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            QueryInformationJobObject,
        };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        let ok = unsafe {
            QueryInformationJobObject(
                self.job,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of_mut!(limits).cast(),
                std::mem::size_of_val(&limits) as u32,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err("QueryInformationJobObject failed".into())
        } else {
            Ok(limits.PeakJobMemoryUsed as u64)
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn memory_bytes(&self) -> Result<u64, String> {
        Err("memory limits are unsupported on this platform".into())
    }

    #[cfg(unix)]
    fn terminate(&self) {
        // Idempotent: once the group has been signalled and the leader reaped, its pgid can
        // be recycled by the OS, so a second kill(-pgid) could hit an UNRELATED group. Do it
        // exactly once. Callers must ensure the pgid is still pinned (the leader alive, or a
        // zombie held via WNOWAIT) at the first call.
        if self.terminated.replace(true) {
            return;
        }
        // Unregister before killing so the signal handler will not also target this id.
        if let Some(slot) = self.track_slot {
            untrack_group(slot);
        }
        let processes = unix_processes().unwrap_or_default();
        // Signal the group ONLY if a live process still holds our pgid. On the kill paths
        // (timeout/memory/output) the leader is alive; on a clean exit with backgrounded
        // work its children still carry the pgid. Either way a live member pins the id, so
        // kill(-pgid) reaches our group and cannot hit a recycled one. When nothing carries
        // the pgid (a clean exit with no leftover work) there is nothing to kill, and
        // skipping is what makes a recycled-id signal impossible (Codex review).
        let group_has_member = processes.iter().any(|p| p.pgid == self.pid);
        unsafe {
            if group_has_member {
                libc::kill(-(self.pid as libc::pid_t), libc::SIGKILL);
            }
            // Belt: also reap any ppid-descendants still findable in this snapshot.
            for pid in descendant_pids(self.pid, &processes) {
                if pid != self.pid {
                    libc::kill(pid as libc::pid_t, libc::SIGKILL);
                }
            }
        }
        // On Linux with a cgroup scope, tear it down: this reaches every process in the
        // scope's cgroup, including a setsid daemon that left the process group and would
        // survive the kill(-pgid) above.
        if let Some(unit) = &self.scope_unit {
            crate::cgroup::kill(unit);
        }
    }

    #[cfg(windows)]
    fn terminate(&self) {
        if self.job.is_null() {
            return;
        }
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn terminate(&self) {}
}

#[cfg(unix)]
#[derive(Clone, Copy)]
struct UnixProcess {
    pid: u32,
    parent: u32,
    pgid: u32,
    rss_kib: u64,
}

#[cfg(unix)]
fn unix_processes() -> Result<Vec<UnixProcess>, String> {
    // Absolute path: a repo that prepends a hijacked or hanging `ps` to PATH must not be
    // able to blind or wedge the memory monitor.
    let output = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid=,rss="])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("ps failed while reading process-tree memory".into());
    }
    // Fail CLOSED on a malformed row: silently dropping one would undercount RSS and could
    // let a tree exceed its ceiling unnoticed (Codex review). A real /bin/ps never emits a
    // bad row, so this only ever fires on a genuinely broken environment.
    let text = String::from_utf8_lossy(&output.stdout);
    let mut processes = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let row = (|| -> Option<UnixProcess> {
            Some(UnixProcess {
                pid: fields.next()?.parse().ok()?,
                parent: fields.next()?.parse().ok()?,
                pgid: fields.next()?.parse().ok()?,
                rss_kib: fields.next()?.parse().ok()?,
            })
        })();
        match row {
            Some(p) => processes.push(p),
            None => return Err(format!("unparseable ps row: {line:?}")),
        }
    }
    Ok(processes)
}

#[cfg(unix)]
fn descendant_pids(root: u32, processes: &[UnixProcess]) -> HashSet<u32> {
    let mut descendants = HashSet::from([root]);
    loop {
        let before = descendants.len();
        for process in processes {
            if descendants.contains(&process.parent) {
                descendants.insert(process.pid);
            }
        }
        if descendants.len() == before {
            return descendants;
        }
    }
}

#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        // terminate() is idempotent and unregisters. If the monitor loop already cleaned up
        // (the common case), this returns immediately and cannot signal a recycled pgid. If
        // it did NOT (a panic before explicit cleanup), the leader is still alive so the
        // pgid is still valid and the kill is safe.
        self.terminate();
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        if self.job.is_null() {
            return;
        }
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::{
        OUTPUT_CAP, UnixProcess, budget_bytes, default_memory_bytes, descendant_pids,
        memory_limit_bytes, over_output_cap, run_program_bounded, run_shell, total_ram_bytes,
    };
    use std::{collections::HashSet, path::Path, time::Duration};

    #[test]
    fn budget_bytes_is_eighty_percent_split_across_concurrency() {
        // 80% of RAM, divided by the concurrency; max(1) guards a zero divisor. Pins the
        // exact ratio so a +/*/÷ swap in the math (mutation survivor) is caught.
        assert_eq!(budget_bytes(1000, 1), 800); // 1000/5*4 = 800
        assert_eq!(budget_bytes(1000, 2), 400); // split across two trees
        assert_eq!(budget_bytes(1000, 0), 800); // max(1): concurrency 0 is treated as 1
        assert_eq!(budget_bytes(0, 4), 0);
    }

    #[test]
    fn over_output_cap_is_a_saturating_sum_over_the_ceiling() {
        assert!(!over_output_cap(0, 0));
        assert!(!over_output_cap(OUTPUT_CAP, 0)); // exactly at the cap is not over
        assert!(over_output_cap(OUTPUT_CAP, 1)); // one byte over
        assert!(over_output_cap(OUTPUT_CAP / 2 + 1, OUTPUT_CAP / 2)); // the SUM crosses
        assert!(!over_output_cap(OUTPUT_CAP / 2, OUTPUT_CAP / 2)); // sum equals cap, not over
        assert!(over_output_cap(u64::MAX, u64::MAX)); // saturates, does not wrap under
    }

    #[test]
    fn run_program_bounded_captures_stdout_and_exit_code() {
        let out = run_program_bounded(
            "sh",
            &["-c", "echo hi; exit 0"],
            Path::new("."),
            Duration::from_secs(5),
        )
        .expect("spawn");
        assert_eq!(out.code, 0);
        assert!(!out.timed_out);
        assert_eq!(out.stdout, "hi\n");
    }

    #[test]
    fn run_program_bounded_reports_nonzero_with_stderr() {
        let out = run_program_bounded(
            "sh",
            &["-c", "echo boom 1>&2; exit 3"],
            Path::new("."),
            Duration::from_secs(5),
        )
        .expect("spawn");
        assert_eq!(out.code, 3);
        assert!(!out.timed_out);
        assert!(out.stderr.contains("boom"), "stderr: {:?}", out.stderr);
    }

    #[test]
    fn run_program_bounded_kills_a_hang_at_the_deadline() {
        // A program that never returns must be killed at the deadline, not hang the caller,
        // and the whole process group must go with it (no orphaned sleep).
        let marker = "crucible-git-hang-probe-9c2e";
        let out = run_program_bounded(
            "sh",
            &["-c", &format!("sleep 30 && echo {marker}")],
            Path::new("."),
            Duration::from_millis(300),
        )
        .expect("spawn");
        assert!(out.timed_out, "a hang must trip the deadline");
        assert_eq!(out.code, 124);
        std::thread::sleep(Duration::from_millis(200));
        let ps = std::process::Command::new("/bin/ps")
            .args(["-axo", "command"])
            .output()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&ps.stdout).contains(marker),
            "the killed program left a survivor in its group"
        );
    }

    // A runaway allocation under a tiny ceiling must be contained — by the kernel cgroup
    // scope where `systemd-run` is available (no poll window), otherwise by the polling
    // fallback. Either way it must be killed on MEMORY, not left to run to the timeout, and
    // never leave the machine tanked. Asserted on every Linux host so there is no silent
    // skip; the scope path is exercised wherever a user systemd session exists.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_runaway_allocation_is_contained_under_a_tiny_ceiling() {
        let output = run_shell(
            "value=0123456789; while :; do value=$value$value; done",
            Path::new("."),
            Duration::from_secs(10),
            Some(32 * 1024 * 1024),
        );
        assert!(
            output.memory_exceeded || output.code != 0,
            "the runaway allocation was not contained (cgroup available: {}): {output:?}",
            crate::cgroup::available(),
        );
        assert!(!output.timed_out, "must die on memory, not the timeout");
    }

    #[test]
    fn a_backgrounded_child_is_killed_when_the_command_returns() {
        // `sleep & ` returns immediately but leaves a grandchild in our process group.
        // A "successful" run must not leave it holding resources — the group is killed on
        // normal exit. We tag the sleep with a unique marker and assert it is gone after.
        let marker = "crucible-bg-leak-probe-4a1f";
        let out = run_shell(
            &format!("(sleep 30 && echo {marker}) & echo started"),
            Path::new("."),
            Duration::from_secs(5),
            None,
        );
        assert_eq!(out.code, 0, "the foreground shell succeeds: {}", out.output);
        // Give the group-kill a moment, then confirm no sleep with our marker survives.
        std::thread::sleep(Duration::from_millis(200));
        let ps = std::process::Command::new("/bin/ps")
            .args(["-axo", "command"])
            .output()
            .unwrap();
        let listing = String::from_utf8_lossy(&ps.stdout);
        assert!(
            !listing.contains(marker),
            "a backgrounded child survived a completed run"
        );
    }

    #[test]
    fn an_absurd_timeout_does_not_panic() {
        // `timeoutSec: u64::MAX` must not overflow the deadline add and orphan the child.
        let out = run_shell(
            "echo ok",
            Path::new("."),
            Duration::from_secs(u64::MAX),
            None,
        );
        assert_eq!(out.code, 0);
        assert_eq!(out.output, "ok\n");
    }

    #[test]
    fn machine_aware_default_composes_with_concurrency() {
        // Total RAM reads on this platform, and the default budget divides by concurrency
        // so N trees stay under the same fraction of RAM.
        let ram = total_ram_bytes().expect("total RAM must be readable on unix test hosts");
        let one = default_memory_bytes(1).unwrap();
        let four = default_memory_bytes(4).unwrap();
        assert!(one < ram, "one tree stays under total RAM");
        assert!(
            four <= one / 3,
            "higher concurrency shrinks the per-tree budget"
        );
        assert!(
            four * 4 <= one,
            "four trees together stay within the one-tree budget"
        );
    }

    #[test]
    fn kills_a_runaway_allocator_at_the_process_tree_budget() {
        let output = run_shell(
            "value=0123456789; while :; do value=$value$value; done",
            Path::new("."),
            Duration::from_secs(5),
            Some(32 * 1024 * 1024),
        );

        assert!(output.memory_exceeded, "{}", output.output);
        assert!(!output.timed_out);
        assert_eq!(output.code, 125);
    }

    #[test]
    fn kills_a_runaway_writer_before_it_fills_the_disk() {
        // `yes` streams unbounded output; the capture file must be capped and the tree
        // killed well before the timeout, protecting the temp partition. No memory limit
        // here, so this proves the disk guard is unconditional and independent of memory.
        let output = run_shell(
            "yes crucible-disk-flood",
            Path::new("."),
            Duration::from_secs(30),
            None,
        );

        assert!(
            output.output_exceeded,
            "should hit the output cap: {}",
            output.output
        );
        assert!(
            !output.timed_out,
            "must die on the disk cap, not the timeout"
        );
        assert!(!output.memory_exceeded);
        assert_eq!(output.code, 126);
        // We only ever hold the tail in memory, never the whole flood.
        assert!(output.output.len() as u64 <= OUTPUT_CAP);
    }

    #[test]
    fn a_well_behaved_command_is_untouched_by_the_guards() {
        let output = run_shell(
            "printf 'hello\\n'",
            Path::new("."),
            Duration::from_secs(5),
            None,
        );
        assert_eq!(output.code, 0);
        assert!(!output.timed_out && !output.memory_exceeded && !output.output_exceeded);
        assert_eq!(output.output, "hello\n");
    }

    #[test]
    fn memory_limit_bytes_rejects_zero_and_converts_mib() {
        assert_eq!(memory_limit_bytes(2048).unwrap(), 2048 * 1024 * 1024);
        assert!(memory_limit_bytes(0).is_err());
    }

    #[test]
    fn descendant_pids_walks_the_parent_chain() {
        let processes = [
            proc(10, 1, 10, 1),
            proc(20, 10, 10, 2),
            proc(30, 20, 10, 3),
            proc(40, 1, 40, 100),
        ];
        assert_eq!(descendant_pids(10, &processes), HashSet::from([10, 20, 30]));
    }

    #[test]
    fn memory_by_group_counts_a_reparented_worker_a_ppid_walk_would_miss() {
        // pid 30's intermediate parent (20) exited, so 30 reparented to init (ppid 1). A
        // PPID walk from the leader (10) would MISS it, but it is still in group 10, so the
        // group-membership sum counts it — matching what kill(-10) reaps. pid 40 is a
        // different group and must not be counted.
        let processes = [
            proc(10, 1, 10, 1),   // group leader (the shell)
            proc(30, 1, 10, 3),   // reparented worker, still pgid 10
            proc(40, 1, 40, 100), // unrelated group
        ];
        let in_group: u64 = processes
            .iter()
            .filter(|p| p.pgid == 10)
            .map(|p| p.rss_kib)
            .sum();
        assert_eq!(in_group, 4, "leader + reparented worker, not the outsider");
        assert_eq!(
            descendant_pids(10, &processes),
            HashSet::from([10]),
            "ppid walk misses 30"
        );
    }

    fn proc(pid: u32, parent: u32, pgid: u32, rss_kib: u64) -> UnixProcess {
        UnixProcess {
            pid,
            parent,
            pgid,
            rss_kib,
        }
    }
}
