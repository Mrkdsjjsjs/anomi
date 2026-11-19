//! Linux: the processor's layout from sysfs, and masks set thread by
//! thread, as Linux keeps a mask for each thread rather than one for the
//! process. A thread starts with the mask of the thread that starts it, and
//! so does a program, through `fork`.

use std::{os::unix::process::CommandExt as _, path::Path};

use super::{Cpu, CpuMask};

/// How a process of Yapfast's hands down the CPUs it started with to the
/// ones it starts, such as a stream worker, whose own start is narrowed
/// already by the mask of the thread that started it: the list, as
/// [`CpuMask`] writes it.
const STARTED_ON: &str = "YAPFAST_STARTED_ON_CPUS";

/// Passes over the process's threads: a thread not yet set may start
/// another as a pass goes.
const PASSES: usize = 3;

/// The nice value of [`lower_priority`].
const LOWER: libc::c_int = 5;

pub(super) fn cpus() -> Vec<Cpu> {
    read(Path::new("/sys"))
}

/// Each logical CPU online and not isolated, from the sysfs at `sys`, with
/// its core and its L3 domain named by their first logical CPU, as
/// `super::number` numbers them.
pub(super) fn read(sys: &Path) -> Vec<Cpu> {
    let base = sys.join("devices/system/cpu");
    let Some(online) = list(&base.join("online")) else {
        return Vec::new();
    };
    let isolated = list(&base.join("isolated")).unwrap_or_default();
    // A perf PMU for each kind of core: only a hybrid has both.
    let small = match list(&sys.join("devices/cpu_atom/cpus")) {
        Some(atoms) if sys.join("devices/cpu_core/cpus").exists() => atoms,
        _ => CpuMask::default(),
    };
    (online.iter())
        .filter(|&cpu| !isolated.contains(cpu))
        .map(|logical| {
            let dir = base.join(format!("cpu{logical}"));
            let siblings = list(&dir.join("topology/core_cpus_list"))
                .or_else(|| list(&dir.join("topology/thread_siblings_list")));
            let core = siblings
                .and_then(|siblings| siblings.iter().next())
                .unwrap_or(logical);
            let (l3, l3_kib) = l3(&dir).unwrap_or((u32::MAX, 0));
            let rank = value(&dir.join("cpufreq/amd_pstate_prefcore_ranking"))
                .or_else(|| value(&dir.join("acpi_cppc/highest_perf")))
                .or_else(|| value(&dir.join("cpufreq/cpuinfo_max_freq")));
            Cpu {
                logical,
                core,
                l3,
                l3_kib,
                rank,
                small: small.contains(logical),
            }
        })
        .collect()
}

/// The L3 cache of the CPU at `dir`: the first CPU sharing it, and its
/// size in KiB.
fn l3(dir: &Path) -> Option<(u32, u32)> {
    let cache = (0..16)
        .map(|index| dir.join(format!("cache/index{index}")))
        .take_while(|cache| cache.exists())
        .find(|cache| text(&cache.join("level")).as_deref() == Some("3"))?;
    let first = list(&cache.join("shared_cpu_list"))?.iter().next()?;
    let size = text(&cache.join("size")).as_deref().and_then(kibibytes);
    Some((first, size.unwrap_or(0)))
}

/// A size as sysfs writes one, `32768K`, in KiB.
fn kibibytes(size: &str) -> Option<u32> {
    let digits = size
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(size.len());
    let (number, unit) = size.split_at(digits);
    let number: u32 = number.parse().ok()?;
    match unit.trim() {
        "K" => Some(number),
        "M" => number.checked_mul(1024),
        "G" => number.checked_mul(1024 * 1024),
        "" => Some(number / 1024),
        _ => None,
    }
}

fn text(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|text| text.trim().to_owned())
}

fn value(path: &Path) -> Option<u32> {
    text(path)?.parse().ok()
}

fn list(path: &Path) -> Option<CpuMask> {
    CpuMask::parse(&text(path)?)
}

/// The CPUs this process started with: as the Yapfast that started it
/// handed them down, else the calling thread's, before Yapfast narrows it.
pub(super) fn start_mask() -> CpuMask {
    if let Some(mask) = handed_down() {
        return mask;
    }
    let mask = current_mask()
        .filter(|mask| !mask.is_empty())
        .or_else(|| list(Path::new("/sys/devices/system/cpu/online")))
        .unwrap_or_default();
    hand_down(&mask);
    mask
}

/// The CPUs [`STARTED_ON`] names, when the process that started this one
/// is this same program: a variable from anywhere else may be stale.
fn handed_down() -> Option<CpuMask> {
    let mask = CpuMask::parse(&std::env::var(STARTED_ON).ok()?)?;
    let ours = std::fs::read_link("/proc/self/exe").ok()?;
    let parent = std::os::unix::process::parent_id();
    let theirs = std::fs::read_link(format!("/proc/{parent}/exe")).ok()?;
    (!mask.is_empty() && ours == theirs).then_some(mask)
}

/// Sets [`STARTED_ON`] for the processes this one starts, while this is
/// its only thread; with more, it doesn't, and they start on the mask they
/// take.
fn hand_down(mask: &CpuMask) {
    let alone = std::fs::read_dir("/proc/self/task").is_ok_and(|threads| threads.count() == 1);
    if mask.is_empty() || !alone {
        return;
    }
    // SAFETY: no other thread runs to read the environment as it changes:
    // this is the process's only one, as just counted.
    unsafe { std::env::set_var(STARTED_ON, mask.to_string()) };
}

/// The calling thread's mask.
pub(super) fn current_mask() -> Option<CpuMask> {
    // SAFETY: an all-zero cpu_set_t is the empty set.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    // SAFETY: 0 is the calling thread, and `set` a whole cpu_set_t to write.
    let read = unsafe { libc::sched_getaffinity(0, size_of::<libc::cpu_set_t>(), &mut set) };
    if read != 0 {
        return None;
    }
    let cpus = (0..CpuMask::LIMIT).filter(|&cpu| {
        // SAFETY: below CpuMask::LIMIT, the set's size.
        unsafe { libc::CPU_ISSET(cpu as usize, &set) }
    });
    Some(cpus.collect())
}

fn cpu_set(mask: &CpuMask) -> libc::cpu_set_t {
    // SAFETY: an all-zero cpu_set_t is the empty set.
    let mut set: libc::cpu_set_t = unsafe { std::mem::zeroed() };
    for cpu in mask.iter() {
        // SAFETY: a mask's CPUs are below CpuMask::LIMIT, the set's size.
        unsafe { libc::CPU_SET(cpu as usize, &mut set) };
    }
    set
}

/// Sets `thread`'s mask; 0 is the calling thread.
fn set_thread(thread: libc::pid_t, mask: &CpuMask) -> std::io::Result<()> {
    let set = cpu_set(mask);
    // SAFETY: `set` is a whole cpu_set_t, which outlives the call.
    let done = unsafe { libc::sched_setaffinity(thread, size_of::<libc::cpu_set_t>(), &set) };
    if done == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Calls `visit` once with each thread of the process, including those
/// started while it goes.
fn each_thread(mut visit: impl FnMut(libc::pid_t)) {
    let mut seen: Vec<libc::pid_t> = Vec::new();
    for _ in 0..PASSES {
        let Ok(tasks) = std::fs::read_dir("/proc/self/task") else {
            return;
        };
        let before = seen.len();
        let threads = (tasks.flatten())
            .filter_map(|task| task.file_name().to_str()?.parse::<libc::pid_t>().ok());
        for thread in threads {
            if !seen.contains(&thread) {
                seen.push(thread);
                visit(thread);
            }
        }
        if seen.len() == before {
            break;
        }
    }
}

/// The name the system shows for `thread`, as `ps` and `top` do.
fn comm(thread: libc::pid_t) -> Option<String> {
    text(Path::new(&format!("/proc/self/task/{thread}/comm")))
}

/// Puts every thread on `target`, or on `start` for none, but threads
/// `free` lists or `names` names on `start`. A thread that ends meanwhile
/// is passed over.
pub(super) fn apply(
    target: Option<&CpuMask>,
    start: &CpuMask,
    free: &[u64],
    names: &[&'static str],
) {
    let narrowed = target.unwrap_or(start);
    if narrowed.is_empty() {
        return;
    }
    each_thread(|thread| {
        let listed = u64::try_from(thread).is_ok_and(|thread| free.contains(&thread));
        let named = !names.is_empty()
            && comm(thread).is_some_and(|comm| names.iter().any(|&name| name == comm));
        let mask = if listed || named { start } else { narrowed };
        if let Err(error) = set_thread(thread, mask)
            && error.raw_os_error() != Some(libc::ESRCH)
        {
            tracing::debug!(thread, %error, "a thread kept its processor mask");
        }
    });
}

pub(super) fn alive(thread: u64) -> bool {
    Path::new(&format!("/proc/self/task/{thread}")).exists()
}

/// Puts the calling thread on `start`, and says its id.
pub(super) fn release_this_thread(start: &CpuMask) -> Option<u64> {
    // SAFETY: gettid has no preconditions.
    let thread = unsafe { libc::gettid() };
    if !start.is_empty()
        && let Err(error) = set_thread(0, start)
    {
        tracing::debug!(%error, "an audio thread kept Yapfast's processor mask");
    }
    u64::try_from(thread).ok()
}

pub(super) fn set_current(mask: &CpuMask) {
    if !mask.is_empty()
        && let Err(error) = set_thread(0, mask)
    {
        tracing::debug!(%error, "a thread kept its processor mask");
    }
}

/// Has `command`'s program start on `start`.
pub(super) fn free_child(command: &mut std::process::Command, start: &CpuMask) {
    if start.is_empty() {
        return;
    }
    let set = cpu_set(start);
    let reset = move || -> std::io::Result<()> {
        // SAFETY: `set` is a whole cpu_set_t; 0 is the child's one thread.
        unsafe { libc::sched_setaffinity(0, size_of::<libc::cpu_set_t>(), &set) };
        Ok(())
    };
    // SAFETY: `reset` makes one system call and allocates nothing, as the
    // child may between `fork` and `exec`.
    unsafe { command.pre_exec(reset) };
}

/// Nice 5 for every thread: Linux keeps a nice value for each.
pub(super) fn lower_priority() {
    each_thread(|thread| {
        let Ok(id) = libc::id_t::try_from(thread) else {
            return;
        };
        // SAFETY: plain values; a process may lower its own threads.
        if unsafe { libc::setpriority(libc::PRIO_PROCESS, id, LOWER) } != 0 {
            let error = std::io::Error::last_os_error();
            tracing::debug!(thread, %error, "a thread kept its priority");
        }
    });
}
