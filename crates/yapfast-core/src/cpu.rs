//! Which of the processor's cores Yapfast runs on, so a game keeps the rest:
//! the processor's layout as the system tells it ([`detect`]), the cores a
//! mode in Settings picks from it ([`choose`]), and the mask every thread of
//! the process is then given ([`apply`]). `docs/cpu.md` says why each rule
//! is as it is, and how to see it on a machine.
//!
//! Nothing here runs on its own: a mask is set once, and the system keeps
//! it. Reading the layout takes 1-2 ms, as Yapfast starts and as the mode
//! changes.

use std::{
    cmp::Reverse,
    fmt,
    sync::{
        Mutex, MutexGuard, OnceLock, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use yapfast_lang::tr;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows as platform;

#[cfg(test)]
mod tests;

/// How Settings has Yapfast choose its cores; `layout.json` keeps it as
/// `cpu_mode`: `"auto"`, `"all"`, or `{"cores": [8, 9, …]}`. The mode is
/// kept rather than the cores, so a new processor, or the folder copied to
/// another computer, gets cores that fit it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CpuMode {
    /// The cores a game is least likely to want ([`choose`]).
    #[default]
    Auto,
    /// Every core the process may use.
    All,
    /// These physical cores, by [`Cpu::core`], each with its SMT sibling.
    Cores(Vec<u32>),
}

/// A mode written in a way this version doesn't know is Auto, so the rest
/// of the file still reads.
impl<'de> Deserialize<'de> for CpuMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Anything else is read past, never looked at.
        #[allow(dead_code)]
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Written {
            Word(String),
            Cores { cores: Vec<u32> },
            Other(serde::de::IgnoredAny),
        }
        Ok(match Written::deserialize(deserializer)? {
            Written::Word(word) if word == "all" => Self::All,
            Written::Cores { cores } if !cores.is_empty() => Self::Cores(cores),
            _ => Self::Auto,
        })
    }
}

/// A set of logical CPUs, by the numbers the system gives them, up to
/// [`CpuMask::LIMIT`] as Linux's `cpu_set_t` holds. It's written as Linux
/// lists CPUs, `8-15,24-31`, in a stream worker's controls and the log.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CpuMask([u64; 16]);

impl CpuMask {
    /// The CPUs a mask can name: numbers below this.
    pub const LIMIT: u32 = 1024;

    /// Adds `cpu`; a number past [`Self::LIMIT`] is left out.
    pub fn insert(&mut self, cpu: u32) {
        if cpu < Self::LIMIT {
            self.0[(cpu / 64) as usize] |= 1u64 << (cpu % 64);
        }
    }

    pub fn contains(&self, cpu: u32) -> bool {
        cpu < Self::LIMIT && self.0[(cpu / 64) as usize] & (1u64 << (cpu % 64)) != 0
    }

    /// How many CPUs it names.
    pub fn len(&self) -> usize {
        self.0.iter().map(|word| word.count_ones() as usize).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&word| word == 0)
    }

    /// Its CPUs, lowest first.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        (0..Self::LIMIT).filter(|&cpu| self.contains(cpu))
    }

    /// The CPUs in both.
    pub fn intersection(&self, other: &Self) -> Self {
        Self(std::array::from_fn(|at| self.0[at] & other.0[at]))
    }

    /// Reads a list as Linux writes one, `0-7,16-23`; none for anything
    /// else, or a CPU past [`Self::LIMIT`]. An empty list is an empty mask.
    pub fn parse(list: &str) -> Option<Self> {
        let mut mask = Self::default();
        for part in list
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let (from, to) = match part.split_once('-') {
                Some((from, to)) => (from.trim().parse().ok()?, to.trim().parse().ok()?),
                None => {
                    let cpu: u32 = part.parse().ok()?;
                    (cpu, cpu)
                }
            };
            if from > to || to >= Self::LIMIT {
                return None;
            }
            (from..=to).for_each(|cpu| mask.insert(cpu));
        }
        Some(mask)
    }

    /// The list for people to read: `8-15, 24-31`.
    pub fn spaced(&self) -> String {
        runs(self.iter(), ", ")
    }
}

impl FromIterator<u32> for CpuMask {
    fn from_iter<I: IntoIterator<Item = u32>>(cpus: I) -> Self {
        let mut mask = Self::default();
        cpus.into_iter().for_each(|cpu| mask.insert(cpu));
        mask
    }
}

impl fmt::Display for CpuMask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&runs(self.iter(), ","))
    }
}

impl fmt::Debug for CpuMask {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "CpuMask({self})")
    }
}

impl Serialize for CpuMask {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CpuMask {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let list = String::deserialize(deserializer)?;
        Self::parse(&list).ok_or_else(|| serde::de::Error::custom("not a list of CPUs"))
    }
}

/// Numbers, lowest first, as runs: `8-15` and `24-31` joined by `separator`.
fn runs(numbers: impl IntoIterator<Item = u32>, separator: &str) -> String {
    fn push(text: &mut String, separator: &str, (from, to): (u32, u32)) {
        if !text.is_empty() {
            text.push_str(separator);
        }
        text.push_str(&from.to_string());
        if to > from {
            text.push('-');
            text.push_str(&to.to_string());
        }
    }
    let mut text = String::new();
    let mut run: Option<(u32, u32)> = None;
    for number in numbers {
        run = match run {
            Some((from, to)) if to.checked_add(1) == Some(number) => Some((from, number)),
            Some(done) => {
                push(&mut text, separator, done);
                Some((number, number))
            }
            None => Some((number, number)),
        };
    }
    if let Some(done) = run {
        push(&mut text, separator, done);
    }
    text
}

/// A logical CPU, as [`detect`] found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cpu {
    /// The system's number for it, as in a mask.
    pub logical: u32,
    /// Its physical core, numbered from 0 in the order of each core's first
    /// logical CPU: SMT siblings share it. On this 9950X3D, cores 0-15, with
    /// CPU n and n + 16 on core n.
    pub core: u32,
    /// Its L3 cache's domain, a chiplet on AMD's processors, numbered from 0
    /// in the same order; one domain where the system names no L3.
    pub l3: u32,
    /// That L3's size, in KiB; 0 when unknown.
    pub l3_kib: u32,
    /// How much the system prefers it for busy threads, higher first:
    /// amd-pstate's preferred-core ranking, else ACPI CPPC's highest
    /// performance, else the highest clock. None on Windows, which doesn't
    /// say.
    pub rank: Option<u32>,
    /// An efficiency core of a processor that also has performance cores,
    /// such as Intel's E-cores.
    pub small: bool,
}

/// A physical core, as [`Topology::cores`] gathers its logical CPUs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Core {
    /// As [`Cpu::core`].
    pub id: u32,
    pub l3: u32,
    pub l3_kib: u32,
    pub rank: Option<u32>,
    pub small: bool,
    /// Its logical CPUs: 2 with SMT.
    pub threads: u32,
}

/// The processor as [`detect`] read it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Topology {
    /// The logical CPUs online and not set aside for the system (Linux's
    /// `isolcpus`), by number.
    pub cpus: Vec<Cpu>,
    /// The CPUs this process started with, before Yapfast narrowed them.
    pub allowed: CpuMask,
}

impl Topology {
    /// The physical cores, by number.
    pub fn cores(&self) -> Vec<Core> {
        let mut cores: Vec<Core> = Vec::new();
        for cpu in &self.cpus {
            match cores.iter_mut().find(|core| core.id == cpu.core) {
                Some(core) => {
                    core.threads += 1;
                    core.rank = core.rank.max(cpu.rank);
                }
                None => cores.push(Core {
                    id: cpu.core,
                    l3: cpu.l3,
                    l3_kib: cpu.l3_kib,
                    rank: cpu.rank,
                    small: cpu.small,
                    threads: 1,
                }),
            }
        }
        cores.sort_by_key(|core| core.id);
        cores
    }

    /// How many physical cores have a logical CPU in `mask`.
    pub fn cores_in(&self, mask: &CpuMask) -> usize {
        let mut cores: Vec<u32> = (self.cpus.iter())
            .filter(|cpu| mask.contains(cpu.logical))
            .map(|cpu| cpu.core)
            .collect();
        cores.sort_unstable();
        cores.dedup();
        cores.len()
    }

    /// The logical CPUs `keep` keeps.
    fn mask(&self, keep: impl Fn(&Cpu) -> bool) -> CpuMask {
        self.cpus
            .iter()
            .filter(|cpu| keep(cpu))
            .map(|cpu| cpu.logical)
            .collect()
    }

    /// Whether someone else, such as `taskset`, a cgroup or a sandbox,
    /// started the process on fewer CPUs than are online.
    fn narrowed(&self) -> bool {
        self.cpus
            .iter()
            .any(|cpu| !self.allowed.contains(cpu.logical))
    }
}

/// Reads the processor's layout from the system: Linux's sysfs, or
/// Windows' CPU sets. Empty where neither is.
pub fn detect() -> Topology {
    let mut cpus = platform::cpus();
    number(&mut cpus);
    Topology {
        cpus,
        allowed: *start(),
    }
}

/// Numbers the cores and the L3 domains from 0, in the order of their
/// first logical CPU, from whatever stands for each in `cpus`.
fn number(cpus: &mut Vec<Cpu>) {
    fn place(seen: &mut Vec<u32>, key: u32) -> u32 {
        let at = seen.iter().position(|&one| one == key).unwrap_or_else(|| {
            seen.push(key);
            seen.len() - 1
        });
        u32::try_from(at).unwrap_or(u32::MAX)
    }
    cpus.sort_by_key(|cpu| cpu.logical);
    cpus.dedup_by_key(|cpu| cpu.logical);
    let (mut cores, mut domains) = (Vec::new(), Vec::new());
    for cpu in cpus.iter_mut() {
        cpu.core = place(&mut cores, cpu.core);
        cpu.l3 = place(&mut domains, cpu.l3);
    }
}

/// Physical cores a chiplet needs for Auto to take it whole: every
/// chiplet of AMD's Zen 3 and later has 6 or 8.
const WHOLE_CHIPLET: usize = 6;

/// Why Auto took the cores it took, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Why {
    /// Nothing was read.
    Unread,
    /// Someone else narrowed the process already.
    Narrowed,
    /// 2 or 3 cores, or 4 without SMT.
    TooFew,
    /// A hybrid processor's efficiency cores.
    Small,
    /// The chiplet with less L3 than core 0's, in MB.
    LessCache(u32),
    /// A chiplet as large as core 0's.
    Chiplet,
    /// The last cores, not core 0.
    Last,
}

/// The logical CPUs `mode` picks, within those the process started with;
/// none when it should keep them all. A pure function of the layout.
pub fn choose(topology: &Topology, mode: &CpuMode) -> Option<CpuMask> {
    match mode {
        CpuMode::Auto => auto(topology).0,
        CpuMode::All => None,
        CpuMode::Cores(cores) => within(topology, topology.mask(|cpu| cores.contains(&cpu.core))),
    }
}

/// `mask` within the CPUs the process started with; none when that's none.
fn within(topology: &Topology, mask: CpuMask) -> Option<CpuMask> {
    let mask = mask.intersection(&topology.allowed);
    (!mask.is_empty()).then_some(mask)
}

/// Auto's rules, in order:
/// - Leave alone a process someone else narrowed, and a processor too
///   small to split.
/// - A hybrid processor: its efficiency cores.
/// - Several L3 domains: one without core 0, with the smallest L3 of
///   those, then the lowest-ranked best core, then the last. Whole when it
///   has [`WHOLE_CHIPLET`] cores; else its last cores, as below.
/// - One domain: the last cores, never core 0, 1 of 4 and then a quarter,
///   at least 2: 2 of 6 or 8, 3 of 12, 4 of 16. "Last" is the lowest
///   ranked, then the highest numbered.
///
/// A core comes with its SMT sibling, so neither shares a core with a
/// game's thread.
fn auto(topology: &Topology) -> (Option<CpuMask>, Why) {
    let Some(first) = topology.cpus.first() else {
        return (None, Why::Unread);
    };
    if topology.narrowed() {
        return (None, Why::Narrowed);
    }
    let cores = topology.cores();
    let smt = cores.iter().any(|core| core.threads > 1);
    if cores.len() < 4 || (cores.len() == 4 && !smt) {
        return (None, Why::TooFew);
    }
    let picked = |mask, why| match within(topology, mask) {
        Some(mask) => (Some(mask), why),
        None => (None, Why::Unread),
    };
    if cores.iter().any(|core| core.small) && cores.iter().any(|core| !core.small) {
        return picked(topology.mask(|cpu| cpu.small), Why::Small);
    }
    let count = last_count(cores.len());
    let mut domains: Vec<u32> = cores.iter().map(|core| core.l3).collect();
    domains.sort_unstable();
    domains.dedup();
    if domains.len() < 2 {
        return picked(last(topology, &cores, count, first.core), Why::Last);
    }
    let size =
        |domain: u32| (cores.iter().find(|core| core.l3 == domain)).map_or(0, |core| core.l3_kib);
    let best = |domain: u32| {
        (cores.iter().filter(|core| core.l3 == domain))
            .filter_map(|core| core.rank)
            .max()
    };
    let mut ours: Vec<u32> = (domains.into_iter())
        .filter(|&domain| domain != first.l3)
        .collect();
    let least = ours.iter().map(|&domain| size(domain)).min();
    ours.retain(|&domain| Some(size(domain)) == least);
    let lowest = ours.iter().map(|&domain| best(domain)).min();
    ours.retain(|&domain| Some(best(domain)) == lowest);
    let Some(&domain) = ours.last() else {
        return (None, Why::Unread);
    };
    let inside: Vec<Core> = cores
        .iter()
        .copied()
        .filter(|core| core.l3 == domain)
        .collect();
    if inside.len() < WHOLE_CHIPLET {
        return picked(last(topology, &inside, count, first.core), Why::Last);
    }
    let why = if size(domain) < size(first.l3) {
        Why::LessCache(size(domain) / 1024)
    } else {
        Why::Chiplet
    };
    picked(topology.mask(|cpu| cpu.l3 == domain), why)
}

/// How many of a processor's `cores` Auto takes from its end.
fn last_count(cores: usize) -> usize {
    if cores <= 4 {
        1
    } else {
        cores.div_ceil(4).max(2)
    }
}

/// The logical CPUs of the last `count` of `cores`, leaving out `zero`:
/// the lowest ranked, then the highest numbered.
fn last(topology: &Topology, cores: &[Core], count: usize, zero: u32) -> CpuMask {
    let mut candidates: Vec<&Core> = cores.iter().filter(|core| core.id != zero).collect();
    candidates.sort_by_key(|core| (core.rank, Reverse(core.id)));
    let taken: Vec<u32> = candidates.iter().take(count).map(|core| core.id).collect();
    topology.mask(|cpu| taken.contains(&cpu.core))
}

/// One line for Settings: what the processor is, then the cores Yapfast
/// runs on with `mask`, as [`choose`] gave it, and why Auto took them.
pub fn describe(topology: &Topology, mask: Option<&CpuMask>) -> String {
    let cores = topology.cores();
    if cores.is_empty() {
        return tr!("Could not read the processor's cores.").to_owned();
    }
    let small = cores.iter().filter(|core| core.small).count();
    let mut domains: Vec<u32> = cores.iter().map(|core| core.l3).collect();
    domains.sort_unstable();
    domains.dedup();
    let found = if small > 0 && small < cores.len() {
        tr!(
            "Performance cores: {0}, efficiency cores: {1}.",
            cores.len() - small,
            small
        )
    } else if domains.len() > 1 {
        tr!(
            "Chiplets: {0}, cores: {1}, threads: {2}.",
            domains.len(),
            cores.len(),
            topology.cpus.len()
        )
    } else {
        tr!(
            "Cores: {0}, threads: {1}.",
            cores.len(),
            topology.cpus.len()
        )
    };
    let (auto, why) = auto(topology);
    let used = match mask.filter(|mask| !mask.is_empty()) {
        None if topology.narrowed() => tr!(
            "Runs on the CPUs the system gave it: {0}.",
            topology.allowed.spaced()
        ),
        None if why == Why::TooFew => {
            tr!("Runs on every core: too few to set some aside.").to_owned()
        }
        None => tr!("Runs on every core.").to_owned(),
        Some(mask) => {
            let mut taken: Vec<u32> = (topology.cpus.iter())
                .filter(|cpu| mask.contains(cpu.logical))
                .map(|cpu| cpu.core)
                .collect();
            taken.sort_unstable();
            taken.dedup();
            let (cores, cpus) = (runs(taken, ", "), mask.spaced());
            match why {
                _ if auto.as_ref() != Some(mask) => {
                    tr!("Runs on cores {0} (CPUs {1}), as chosen.", cores, cpus)
                }
                Why::LessCache(megabytes) => tr!(
                    "Runs on cores {0} (CPUs {1}): the chiplet with the smaller cache, {2} MB.",
                    cores,
                    cpus,
                    megabytes
                ),
                Why::Chiplet => tr!(
                    "Runs on cores {0} (CPUs {1}): the chiplet without core 0.",
                    cores,
                    cpus
                ),
                Why::Small => tr!(
                    "Runs on cores {0} (CPUs {1}): the efficiency cores.",
                    cores,
                    cpus
                ),
                _ => tr!(
                    "Runs on cores {0} (CPUs {1}): the last ones, core 0 stays free.",
                    cores,
                    cpus
                ),
            }
        }
    };
    format!("{found} {used}")
}

/// The CPUs this process started with: on Linux as a process of Yapfast's
/// that started it handed them down, else as the system gave them; read
/// once, before Yapfast narrows them.
fn start() -> &'static CpuMask {
    static START: OnceLock<CpuMask> = OnceLock::new();
    START.get_or_init(platform::start_mask)
}

/// What [`apply`] keeps track of between calls.
struct Kept {
    /// Threads that keep every CPU the process started with: audio threads
    /// that didn't get real-time priority ([`release_this_thread`]).
    threads: Vec<u64>,
    /// Threads that do so by name ([`keep_free`]).
    names: Vec<&'static str>,
    /// The mask set; none for every CPU the process started with.
    applied: Option<CpuMask>,
}

static KEPT: Mutex<Kept> = Mutex::new(Kept {
    threads: Vec::new(),
    names: Vec::new(),
    applied: None,
});

fn kept() -> MutexGuard<'static, Kept> {
    KEPT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Logical CPUs the process runs on as [`apply`] last left it: the mask's,
/// or for none, as the system counted them then; 0 before it ran. Counted
/// once there, so [`threads`] reads a number, as a stream worker does for
/// each frame x264 encodes.
static CHOSEN: AtomicUsize = AtomicUsize::new(0);

/// Puts every thread of this process on `mask`, now, or back on every CPU
/// it started with for none; each thread started after takes the mask of
/// the one that starts it. Audio threads without real-time priority, and
/// threads [`keep_free`] names, keep every CPU. Linux sets each thread's
/// own mask, about 100 calls; Windows sets the process's default CPU sets.
pub fn apply(mask: Option<&CpuMask>) {
    let start = start();
    let target = mask.filter(|mask| !mask.is_empty()).copied();
    let mut kept = kept();
    kept.threads.retain(|&thread| platform::alive(thread));
    platform::apply(target.as_ref(), start, &kept.threads, &kept.names);
    let cpus = target.map_or_else(available, |mask| mask.len());
    CHOSEN.store(cpus, Ordering::Relaxed);
    match &target {
        Some(mask) => tracing::info!(cpus = %mask, "Yapfast runs on these logical CPUs"),
        None => tracing::info!(cpus = %start, "Yapfast runs on every logical CPU it started with"),
    }
    kept.applied = target;
}

/// The most threads a decoder or an encoder takes.
const MOST_THREADS: usize = 4;

/// Threads for a video decoder or encoder: one for each logical CPU the
/// process runs on, at most 4 ([`threads_on`]). On Windows the standard
/// library's count doesn't see CPU sets, so this counts the mask applied.
pub fn threads() -> usize {
    threads_on(match CHOSEN.load(Ordering::Relaxed) {
        0 => available(),
        cpus => cpus,
    })
}

/// The logical CPUs the calling thread may use, as the standard library
/// counts them.
fn available() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get)
}

/// Threads for a video decoder or encoder on `cpus` logical CPUs: one for
/// each, at least 1 and at most 4. A stream's x264 takes these for the
/// cores chosen, never the machine's.
pub fn threads_on(cpus: usize) -> usize {
    cpus.clamp(1, MOST_THREADS)
}

/// Sizes the thread pools Yapfast's libraries would otherwise size to
/// every logical CPU: rayon's, which openmls starts for DAVE's group
/// changes, 32 idle threads here, gets 2 named `yapfast-mls-N`. Call it
/// once as the program starts, before a call.
pub fn init_pools() {
    #[cfg(feature = "discord")]
    mls_pool();
}

#[cfg(feature = "discord")]
fn mls_pool() {
    let built = rayon_core::ThreadPoolBuilder::new()
        .num_threads(2)
        .thread_name(|index| format!("yapfast-mls-{index}"))
        .build_global();
    if let Err(error) = built {
        tracing::debug!(%error, "rayon's pool was already started");
    }
}

/// Lets the calling thread run on every CPU the process started with, now
/// and through each later [`apply`]: an audio thread whose priority
/// couldn't be raised, so it needn't wait beside a game's threads for one
/// of Yapfast's cores.
pub fn release_this_thread() {
    let start = start();
    let mut kept = kept();
    kept.threads.retain(|&thread| platform::alive(thread));
    if let Some(thread) = platform::release_this_thread(start)
        && !kept.threads.contains(&thread)
    {
        kept.threads.push(thread);
    }
}

/// Has threads named `name`, as the system shows the name, keep every CPU
/// the process started with through each later [`apply`]: libpulse's,
/// which carry a stream's samples at normal priority. A thread takes the
/// name as it starts, so start it within [`unconfined`] too.
pub fn keep_free(name: &'static str) {
    let mut kept = kept();
    if !kept.names.contains(&name) {
        kept.names.push(name);
    }
}

/// Runs `work` with the calling thread on every CPU the process started
/// with, and puts it back after: threads and programs it starts take that
/// mask rather than Yapfast's. Linux only; elsewhere it just runs `work`.
pub fn unconfined<R>(work: impl FnOnce() -> R) -> R {
    let start = start();
    let Some(own) = platform::current_mask().filter(|own| own != start) else {
        return work();
    };
    platform::set_current(start);
    let result = work();
    platform::set_current(&own);
    result
}

/// Has the program `command` starts run on every CPU this process started
/// with, rather than the mask of the thread that starts it: a browser that
/// `xdg-open` starts. Linux only.
pub fn free_child(command: &mut std::process::Command) {
    let start = start();
    if platform::current_mask().is_some_and(|own| own != *start) {
        platform::free_child(command, start);
    }
}

/// Systems where Yapfast reads no layout and sets no mask.
#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::{Cpu, CpuMask};

    pub(super) fn cpus() -> Vec<Cpu> {
        Vec::new()
    }

    pub(super) fn start_mask() -> CpuMask {
        CpuMask::default()
    }

    pub(super) fn apply(_: Option<&CpuMask>, _: &CpuMask, _: &[u64], _: &[&'static str]) {}

    pub(super) fn alive(_: u64) -> bool {
        false
    }

    pub(super) fn release_this_thread(_: &CpuMask) -> Option<u64> {
        None
    }

    pub(super) fn current_mask() -> Option<CpuMask> {
        None
    }

    pub(super) fn set_current(_: &CpuMask) {}

    pub(super) fn free_child(_: &mut std::process::Command, _: &CpuMask) {}
}
