# Resource safety

The four arms that spawn a real build or test tree — `run`, `harden`, `cover`, `flake` —
are bounded so parallel sessions cannot take the machine down:

- **Machine-wide concurrency gate.** Only N heavy runs execute at once across *every*
  Crucible process on the box (default `1`, capped at core count). Set it persistently
  with `crucible config max-concurrency <N>` — a machine-level file at
  `~/.config/crucible/config.json`, deliberately not repo config, so an agent cannot
  grant itself more slots by editing the repo it is working in. `CRUCIBLE_MAX_CONCURRENCY` overrides it per-shell,
  and `crucible config` / `crucible doctor` show which layer is live. More slots also
  means a lower default memory ceiling per tree (the budget below splits N ways). A
  session that arrives while the slots are full waits (up to `CRUCIBLE_SLOT_WAIT_SECS`,
  default one hour) rather than piling another `cargo` build onto the memory. Slots are OS
  file locks in a fixed machine-wide path (`/tmp/crucible-slots` on unix), so a crashed
  session frees its slot automatically and a per-session `TMPDIR` does not split the
  namespace. `CRUCIBLE_SLOTS_DIR` overrides the path — a deliberate escape hatch for
  per-project gating; setting a different value per session splits the gate, so only do it
  on purpose. If a second `crucible` seems to hang, it is queued.
- **Kernel-enforced containment on Linux.** Where `systemd-run --user --scope` is available,
  each heavy tree runs in a cgroup-v2 scope: `MemoryMax` OOM-kills it the instant it exceeds
  (no poll window), `MemorySwapMax=0` stops swapping around the limit, `TasksMax` caps the
  process count (fork-bomb protection), and tearing down the scope reaches every process in
  its cgroup — including a `setsid` daemon that left the process group. This holds even for
  an orphaned tree: the kernel cap means it cannot tank RAM. Set `CRUCIBLE_NO_CGROUP=1` to
  force the fallback below. (macOS/Windows and non-systemd Linux use the fallback.)
- **Process-tree memory ceiling (fallback + belt).** `run`, `cover`, and `flake` cap their
  tree at a machine-aware default (a fraction of RAM, divided by the concurrency so N
  same-configured trees stay within budget) unless a recipe sets an explicit `memoryMb`;
  `harden` uses a fixed 2048 MiB default, also overridable. The ceiling is measured by
  summing the process group's RSS (a reparented worker still in the group is counted). It is
  *polled*: sustained overuse is killed within the sample interval. Where the cgroup scope is
  active the kernel is the hard backstop; where it is not, a command that allocates several
  GiB between samples can spike — set `memoryMb` low for known-heavy recipes. If total RAM
  cannot be read, the arm runs uncapped and says so.
- **Hard timeout** with whole-process-tree kill (group/JobObject) on every exit path,
  including any work a recipe backgrounds with `&`.
- **Log-capture cap.** The captured stdout/stderr file is polled and the tree killed once
  it crosses ~256 MiB (a fast writer can overshoot by one 20 ms sample, not grow without
  bound). This bounds the *capture file*, not arbitrary files a command writes elsewhere
  (`dd of=big.bin` is not caught) — scope untrusted recipes with a filesystem quota.

A run killed by any cap fails closed — it certifies nothing.

### Containment by platform

| Threat | Linux + `systemd-run` scope | Fallback (macOS, Windows, non-systemd Linux) |
| --- | --- | --- |
| One tree exhausts RAM | Kernel `MemoryMax` OOM-kills it, no poll window | Polled RSS ceiling; a burst between samples can spike |
| Fork bomb | Kernel `TasksMax` | Whole-process-group kill on cleanup |
| `setsid` / double-fork escape | Scope cgroup kill reaches it | **Escapes** the process-group kill |
| Crucible itself `SIGKILL`ed | Orphan survives but is **still RAM-capped** by the cgroup | Orphan survives and is **uncapped** |
| Crucible `SIGTERM`/`SIGINT`/`SIGHUP` | Group + scope torn down | Group torn down (setsid escapes survive) |

On Linux with a user systemd session the first four are contained by the kernel. The
fallback path bounds the realistic case (honest recipes, RAM readable) but leaves the
escapes above — stated rather than hidden.

Remaining on every platform:

- **Windows spawn-before-job window.** A grandchild started in the brief window before the
  Job Object is assigned is not retroactively contained.
- **Diff-discovery git is PATH-resolved.** `harden`/`cover` shell out to `git diff` (from
  PATH) *before* acquiring their slot. That git now runs under a 60-second hard timeout with
  a whole-process-group kill and an output cap, so a hung or flooding `git` can no longer
  hang Crucible or fill the disk outside the gate — but a hostile PATH `git` still runs with
  your privileges. Trust your PATH, or run Crucible where `git` is fixed.

