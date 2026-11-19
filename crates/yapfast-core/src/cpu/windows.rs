//! Windows: the processor's layout from its CPU sets, and the process's
//! default CPU sets, which Windows keeps for every thread that chose none.
//! CPU sets are soft: Windows may still run a thread elsewhere when it
//! must, and they stay out of the way of core parking and Game Mode, where
//! a hard affinity mask would not.
//!
//! UNTESTED: written to compile, and not run on Windows. Unknown there are
//! whether threads already running move at once, whether AMD's driver
//! parking the frequency chiplet in a game moves Yapfast onto the game's,
//! and how efficiency cores and EcoQoS meet (`docs/cpu.md`).

use ::windows::Win32::{
    Foundation::HANDLE,
    System::{
        SystemInformation::{
            CACHE_RELATIONSHIP, CpuSetInformation, GROUP_AFFINITY,
            GetLogicalProcessorInformationEx, GetSystemCpuSetInformation, RelationCache,
            SYSTEM_CPU_SET_INFORMATION, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
        },
        Threading::{
            BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, GetCurrentThread, GetCurrentThreadId,
            GetProcessAffinityMask, SetPriorityClass, SetProcessAffinityMask,
            SetProcessDefaultCpuSets, SetThreadSelectedCpuSets,
        },
    },
};

use super::{Cpu, CpuMask};

/// A CPU set's flags: reserved for a process, such as a game in Game Mode.
const ALLOCATED: u8 = 0x2;
/// Reserved for this process.
const ALLOCATED_HERE: u8 = 0x4;

/// One CPU set, a logical CPU.
struct Set {
    id: u32,
    logical: u32,
    /// Its core and its last-level cache, each within its processor group.
    core: (u16, u8),
    cache: (u16, u8),
    efficiency: u8,
    /// Reserved for another process.
    theirs: bool,
}

/// The system's CPU sets.
fn sets() -> Vec<Set> {
    // SAFETY: the current process's pseudo handle needs no closing.
    let process = unsafe { GetCurrentProcess() };
    let mut length = 0u32;
    // SAFETY: no buffer, so it only says the length needed, and fails.
    let _ = unsafe { GetSystemCpuSetInformation(None, 0, &mut length, Some(process), None) };
    if length == 0 {
        return Vec::new();
    }
    let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
    // SAFETY: the buffer holds `length` bytes, aligned for the records.
    let read = unsafe {
        GetSystemCpuSetInformation(
            Some(buffer.as_mut_ptr().cast()),
            length,
            &mut length,
            Some(process),
            None,
        )
    };
    if !read.as_bool() {
        return Vec::new();
    }
    let bytes = (length as usize).min(buffer.len() * 8);
    let base = buffer.as_ptr().cast::<u8>();
    let mut sets = Vec::new();
    let mut at = 0;
    while at + size_of::<SYSTEM_CPU_SET_INFORMATION>() <= bytes {
        // SAFETY: a whole record lies at `at`, within the buffer.
        let record = unsafe {
            base.add(at)
                .cast::<SYSTEM_CPU_SET_INFORMATION>()
                .read_unaligned()
        };
        if record.Size == 0 {
            break;
        }
        if record.Type == CpuSetInformation {
            // SAFETY: a CPU set record's union holds a CPU set, whose flags
            // are a byte either way.
            let (set, flags) = unsafe {
                (
                    record.Anonymous.CpuSet,
                    record.Anonymous.CpuSet.Anonymous1.AllFlags,
                )
            };
            sets.push(Set {
                id: set.Id,
                logical: u32::from(set.Group) * 64 + u32::from(set.LogicalProcessorIndex),
                core: (set.Group, set.CoreIndex),
                cache: (set.Group, set.LastLevelCacheIndex),
                efficiency: set.EfficiencyClass,
                theirs: flags & ALLOCATED != 0 && flags & ALLOCATED_HERE == 0,
            });
        }
        at += record.Size as usize;
    }
    sets
}

/// Each logical CPU's L3 size in KiB, from the system's caches.
fn l3_sizes() -> Vec<(u32, u32)> {
    let mut length = 0u32;
    // SAFETY: no buffer, so it only says the length needed, and fails.
    let _ = unsafe { GetLogicalProcessorInformationEx(RelationCache, None, &mut length) };
    if length == 0 {
        return Vec::new();
    }
    let mut buffer = vec![0u64; (length as usize).div_ceil(8)];
    // SAFETY: the buffer holds `length` bytes, aligned for the entries.
    let read = unsafe {
        GetLogicalProcessorInformationEx(
            RelationCache,
            Some(buffer.as_mut_ptr().cast()),
            &mut length,
        )
    };
    if read.is_err() {
        return Vec::new();
    }
    let bytes = (length as usize).min(buffer.len() * 8);
    let base = buffer.as_ptr().cast::<u8>();
    let header = std::mem::offset_of!(SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX, Anonymous);
    let masks = header + std::mem::offset_of!(CACHE_RELATIONSHIP, Anonymous);
    let mut sizes = Vec::new();
    let mut at = 0;
    while at + header <= bytes {
        // SAFETY: an entry starts with its relationship and its size, within
        // the buffer.
        let (relationship, size) = unsafe {
            (
                base.add(at).cast::<i32>().read_unaligned(),
                base.add(at + 4).cast::<u32>().read_unaligned() as usize,
            )
        };
        if size == 0 || at + size > bytes {
            break;
        }
        if relationship == RelationCache.0 && header + size_of::<CACHE_RELATIONSHIP>() <= size {
            // SAFETY: the entry holds a whole cache record, as just checked.
            let cache = unsafe {
                (base.add(at + header))
                    .cast::<CACHE_RELATIONSHIP>()
                    .read_unaligned()
            };
            for group in 0..usize::from(cache.GroupCount.max(1)) {
                let offset = masks + group * size_of::<GROUP_AFFINITY>();
                if cache.Level != 3 || offset + size_of::<GROUP_AFFINITY>() > size {
                    break;
                }
                // SAFETY: within the entry, as just checked.
                let mask = unsafe {
                    (base.add(at + offset))
                        .cast::<GROUP_AFFINITY>()
                        .read_unaligned()
                };
                for bit in (0..usize::BITS).filter(|&bit| mask.Mask & (1usize << bit) != 0) {
                    sizes.push((u32::from(mask.Group) * 64 + bit, cache.CacheSize / 1024));
                }
            }
        }
        at += size;
    }
    sizes
}

/// Each logical CPU, with its core and its L3 domain named by their first
/// logical CPU. Windows gives no preferred-core ranking.
pub(super) fn cpus() -> Vec<Cpu> {
    let sets = sets();
    let sizes = l3_sizes();
    let least = sets.iter().map(|set| set.efficiency).min();
    let most = sets.iter().map(|set| set.efficiency).max();
    (sets.iter())
        .map(|set| Cpu {
            logical: set.logical,
            core: first(&sets, |other| other.core == set.core).unwrap_or(set.logical),
            l3: first(&sets, |other| other.cache == set.cache).unwrap_or(set.logical),
            l3_kib: (sizes.iter())
                .find(|(logical, _)| *logical == set.logical)
                .map_or(0, |(_, kib)| *kib),
            rank: None,
            // Intel's E-cores are the lowest class where classes differ.
            small: least != most && Some(set.efficiency) == least,
        })
        .collect()
}

/// The lowest logical CPU of the sets `same` keeps.
fn first(sets: &[Set], same: impl Fn(&Set) -> bool) -> Option<u32> {
    sets.iter()
        .filter(|set| same(set))
        .map(|set| set.logical)
        .min()
}

/// The CPUs the process may use, from its affinity mask within one
/// processor group; past 64 CPUs, all of them.
pub(super) fn start_mask() -> CpuMask {
    let all: CpuMask = sets().iter().map(|set| set.logical).collect();
    let (mut process, mut system) = (0usize, 0usize);
    // SAFETY: the pseudo handle, and two values to write.
    let read = unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) };
    if read.is_err() || process == 0 || all.iter().any(|cpu| cpu >= usize::BITS) {
        return all;
    }
    all.iter()
        .filter(|&cpu| process & (1usize << cpu) != 0)
        .collect()
}

/// The ids of the CPU sets in `mask` no other process holds, or all those
/// in `mask` when others hold every one.
fn ids(mask: &CpuMask) -> Vec<u32> {
    let sets = sets();
    let mine: Vec<u32> = (sets.iter())
        .filter(|set| mask.contains(set.logical) && !set.theirs)
        .map(|set| set.id)
        .collect();
    if !mine.is_empty() {
        return mine;
    }
    (sets.iter())
        .filter(|set| mask.contains(set.logical))
        .map(|set| set.id)
        .collect()
}

/// Sets the process's default CPU sets to `target`, or clears them for
/// none; where Windows refuses, the hard affinity mask, which only works
/// within one processor group and which programs started after take too.
/// Threads [`release_this_thread`] freed chose their own sets already.
pub(super) fn apply(
    target: Option<&CpuMask>,
    start: &CpuMask,
    _free: &[u64],
    _names: &[&'static str],
) {
    // SAFETY: the current process's pseudo handle.
    let process = unsafe { GetCurrentProcess() };
    let Some(target) = target else {
        // SAFETY: no ids clears the default.
        let _ = unsafe { SetProcessDefaultCpuSets(process, None) };
        hard(process, start);
        return;
    };
    let ids = ids(target);
    // SAFETY: ids of this machine's CPU sets.
    if unsafe { SetProcessDefaultCpuSets(process, Some(ids.as_slice())) }.as_bool() {
        return;
    }
    tracing::info!("Windows refused CPU sets; Yapfast sets its affinity mask instead");
    hard(process, target);
}

fn hard(process: HANDLE, mask: &CpuMask) {
    if mask.is_empty() || mask.iter().any(|cpu| cpu >= usize::BITS) {
        return;
    }
    let bits = mask.iter().fold(0usize, |bits, cpu| bits | (1usize << cpu));
    // SAFETY: the pseudo handle and a mask of this group's processors.
    if let Err(error) = unsafe { SetProcessAffinityMask(process, bits) } {
        tracing::debug!(%error, "Windows kept Yapfast's affinity mask");
    }
}

/// Windows has no way here to tell a thread id that ended from one reused;
/// the list stays short, as each call frees three threads at most.
pub(super) fn alive(_: u64) -> bool {
    true
}

/// Has the calling thread choose every CPU set `start` names, which no
/// later default changes.
pub(super) fn release_this_thread(start: &CpuMask) -> Option<u64> {
    let ids = ids(start);
    if !ids.is_empty() {
        // SAFETY: the calling thread's pseudo handle, and ids of this
        // machine's CPU sets.
        let chosen = unsafe { SetThreadSelectedCpuSets(GetCurrentThread(), &ids) };
        if !chosen.as_bool() {
            tracing::debug!("an audio thread kept Yapfast's CPU sets");
        }
    }
    // SAFETY: no preconditions.
    Some(u64::from(unsafe { GetCurrentThreadId() }))
}

/// A thread's own CPU sets don't pass to the programs it starts.
pub(super) fn current_mask() -> Option<CpuMask> {
    None
}

pub(super) fn set_current(_: &CpuMask) {}

pub(super) fn free_child(_: &mut std::process::Command, _: &CpuMask) {}

pub(super) fn lower_priority() {
    // SAFETY: the current process's pseudo handle.
    if let Err(error) =
        unsafe { SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS) }
    {
        tracing::debug!(%error, "Windows kept Yapfast's priority");
    }
}
