//! Sound through PulseAudio, or PipeWire's PulseAudio server.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::{
        Once,
        atomic::{AtomicI32, AtomicU64, Ordering},
    },
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow, bail};
use libpulse_binding::{
    callbacks::ListResult,
    context::{self, Context, FlagSet as ContextFlags},
    def::BufferAttr,
    mainloop::standard::{IterateResult, Mainloop},
    sample::{Format as PulseFormat, Spec},
    stream::Direction,
};
use libpulse_simple_binding::Simple;

use super::Format;
use crate::voice::NAMED;

/// Real-time priority for Yapfast's audio threads: above every ordinary
/// thread, and below PipeWire's own, which run at 88 or more. A
/// microphone's thread runs a step below those that play.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
const REALTIME: i32 = 10;

/// The name libpulse gives the thread that carries each stream's samples
/// to the server, at normal priority.
const STREAM_THREAD: &str = "threaded-ml";

/// RealtimeKit, on the system bus: it raises a thread of the asking process
/// to real-time priority where the user's limits don't let the process do
/// it itself, as PipeWire's clients ask it.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
const RTKIT: &str = "org.freedesktop.RealtimeKit1";
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
const RTKIT_PATH: &str = "/org/freedesktop/RealtimeKit1";

/// How long the audio thread waits for rtkit's answer before it carries
/// on at normal priority.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
const RTKIT_WAIT: Duration = Duration::from_secs(2);

/// Says how it raised the calling thread, or why it couldn't: by itself
/// where the limits allow it (`ulimit -r`), else through rtkit. The thread
/// is watched first ([`watch_this_thread`]), so SIGXCPU's handler can put it
/// back at normal priority. Called again for a thread the handler put back,
/// it raises it again.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub fn raise_priority(priority: super::Priority) -> Result<&'static str, String> {
    let (sched_priority, how, through_rtkit) = match priority {
        super::Priority::Playback => (REALTIME, "SCHED_FIFO 10", "SCHED_RR 10 through rtkit"),
        super::Priority::Capture => (REALTIME - 1, "SCHED_FIFO 9", "SCHED_RR 9 through rtkit"),
    };
    if !watch_this_thread() {
        return Err(format!(
            "{WATCHED} audio threads run at real-time priority already"
        ));
    }
    // Limits that allow real-time priority may limit its time too: then
    // SIGXCPU puts the thread back, as for a thread rtkit raised. Watched
    // before the raise, as the thread isn't counted yet.
    let limited = limit_real_time(None);
    let param = libc::sched_param { sched_priority };
    // SAFETY: 0 is the calling thread, and `param` outlives the call. Threads
    // it starts later begin at normal priority again.
    let raised = unsafe {
        libc::sched_setscheduler(0, libc::SCHED_FIFO | libc::SCHED_RESET_ON_FORK, &param)
    };
    if raised == 0 {
        if let Err(error) = limited {
            tracing::warn!(%error, "an audio thread that runs too long could end the program");
        }
        // Linux counts a real-time thread's time from when it last woke at
        // real-time priority, and a thread put back at normal priority keeps
        // what it had counted: raised again, it starts from nothing once it
        // has slept, as through rtkit, whose answer it waits for.
        std::thread::sleep(Duration::from_millis(1));
        return Ok(how);
    }
    let refused = std::io::Error::last_os_error();
    match rtkit(sched_priority) {
        Ok(given) if given == sched_priority => Ok(through_rtkit),
        Ok(_) => Ok("SCHED_RR through rtkit, below the priority asked"),
        // An answer that came too late may follow a raise: the thread stays
        // watched while it's real-time.
        Err(_) if real_time() => Ok("SCHED_RR through rtkit, which answered late"),
        Err(error) => {
            forget_this_thread();
            Err(format!("{refused}; {error}"))
        }
    }
}

/// Puts the calling thread back at normal priority, for good: SIGXCPU's
/// handler no longer watches it.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub fn lower_priority() {
    let normal = libc::sched_param { sched_priority: 0 };
    // SAFETY: 0 is the calling thread, and `normal` outlives the call. It
    // keeps SCHED_RESET_ON_FORK, as only a privileged thread may clear it.
    unsafe {
        libc::sched_setscheduler(0, libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK, &normal);
    }
    forget_this_thread();
}

/// Whether the calling thread runs at real-time priority.
fn real_time() -> bool {
    // SAFETY: 0 is the calling thread; there are no other preconditions.
    let policy = unsafe { libc::sched_getscheduler(0) } & !libc::SCHED_RESET_ON_FORK;
    policy == libc::SCHED_FIFO || policy == libc::SCHED_RR
}

/// Asks rtkit to put the calling thread on SCHED_RR at `priority`, or the
/// highest it allows below that, and says which. rtkit raises a thread
/// only for a process whose RLIMIT_RTTIME is within its own limit, 200 ms
/// here, so this sets it first: a real-time thread of Yapfast's that ran
/// that long without once waiting would end the process rather than hold
/// a core. rtkit sets SCHED_RESET_ON_FORK too: threads started after begin
/// at normal priority.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
fn rtkit(priority: i32) -> Result<i32, String> {
    use zbus::zvariant::{OwnedValue, Value};
    let bus = zbus::blocking::connection::Builder::system()
        .map(|builder| builder.method_timeout(RTKIT_WAIT))
        .and_then(|builder| builder.build())
        .map_err(|error| format!("no system bus for rtkit: {error}"))?;
    let property = |name: &str| -> Result<i64, String> {
        let reply = bus
            .call_method(
                Some(RTKIT),
                RTKIT_PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(RTKIT, name),
            )
            .and_then(|reply| reply.body().deserialize::<OwnedValue>())
            .map_err(|error| format!("rtkit: {error}"))?;
        match &*reply {
            Value::I32(value) => Ok(i64::from(*value)),
            Value::I64(value) => Ok(*value),
            _ => Err(format!("rtkit gave {name} as something else than a number")),
        }
    };
    let priority = i64::from(priority).min(property("MaxRealtimePriority")?);
    let priority = u32::try_from(priority)
        .ok()
        .filter(|&priority| priority > 0)
        .ok_or("rtkit allows no real-time priority")?;
    limit_real_time(Some(property("RTTimeUSecMax")?))?;
    // SAFETY: gettid has no preconditions.
    let thread = unsafe { libc::gettid() };
    let thread = u64::try_from(thread).map_err(|error| error.to_string())?;
    bus.call_method(
        Some(RTKIT),
        RTKIT_PATH,
        Some(RTKIT),
        "MakeThreadRealtime",
        &(thread, priority),
    )
    .map_err(|error| format!("rtkit: {error}"))?;
    i32::try_from(priority).map_err(|error| error.to_string())
}

/// Watches the processor time a real-time thread takes without waiting,
/// the process's RLIMIT_RTTIME, in microseconds: lowers the hard limit to
/// `most` where given and above it, and the soft one to half the hard. A
/// thread's count starts again each time it wakes at real-time priority
/// from a sleep, and Linux counts it in
/// ticks of the scheduler's clock, a millisecond at 1000 Hz. Past the hard
/// limit Linux kills the whole program, with SIGKILL; at the soft one it
/// sends SIGXCPU, on which [`put_back`] puts every raised thread back at
/// normal priority, where the count stops. A thread held up by a long job
/// stutters, rather than ends the call, with 100 ms of its own time left
/// between the two under rtkit's 200 ms. While neither limit is set,
/// nothing changes and nothing is watched: no signal comes.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
fn limit_real_time(most: Option<i64>) -> Result<(), String> {
    let most = most
        .map(|most| libc::rlim_t::try_from(most).map_err(|_| "rtkit gave no time limit".to_owned()))
        .transpose()?;
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `limit` is a whole rlimit for the call to write.
    if unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &mut limit) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let hard = most.map_or(limit.rlim_max, |most| limit.rlim_max.min(most));
    let soft = if hard == libc::RLIM_INFINITY {
        limit.rlim_cur
    } else {
        limit.rlim_cur.min((hard / 2).max(1))
    };
    if soft == libc::RLIM_INFINITY {
        return Ok(());
    }
    watch_real_time(soft);
    if (limit.rlim_cur, limit.rlim_max) == (soft, hard) {
        return Ok(());
    }
    let limit = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    // SAFETY: `limit` outlives the call; lowering a limit needs no privilege.
    if unsafe { libc::setrlimit(libc::RLIMIT_RTTIME, &limit) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

/// The soft RLIMIT_RTTIME [`limit_real_time`] set, which [`put_back`] sets
/// again: Linux moves it a second on as it signals, which, past the hard
/// limit, would leave the next thread that runs too long no signal before
/// it's killed.
static SOFT_LIMIT: AtomicU64 = AtomicU64::new(0);

/// Threads raised to real-time priority, as many at once as are watched:
/// a call's three, the microphone test's two beside them, and room to spare.
const WATCHED: usize = 8;
/// The thread ids of the threads raised, 0 in a free slot, which
/// [`put_back`] lowers.
static RAISED: [AtomicI32; WATCHED] = [const { AtomicI32::new(0) }; WATCHED];
/// What [`put_back`] did to each slot's thread since the thread last
/// looked ([`lowered`]): [`UNTOUCHED`], [`LOWERED`], [`RAN_LONG`], or the
/// error number it was refused with.
static PUT: [AtomicI32; WATCHED] = [const { AtomicI32::new(UNTOUCHED) }; WATCHED];
const UNTOUCHED: i32 = 0;
/// Put back beside the thread that ran too long.
const LOWERED: i32 = -1;
/// Put back as the thread that ran too long: the one the handler ran on,
/// as Linux has the thread that ran past the limit take the signal first
/// (`complete_signal`), or each, should it run on a thread not watched.
const RAN_LONG: i32 = -2;

thread_local! {
    /// The slot in [`RAISED`] the calling thread holds, freed as it ends.
    static SLOT: Slot = const { Slot(Cell::new(None)) };
}

struct Slot(Cell<Option<usize>>);

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(slot) = self.0.take() {
            free(slot);
        }
    }
}

fn free(slot: usize) {
    RAISED[slot].store(0, Ordering::Release);
    PUT[slot].store(UNTOUCHED, Ordering::Relaxed);
}

/// Has [`put_back`] lower the calling thread, from now until it ends or
/// [`forget_this_thread`]; whether a slot was free for it.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
fn watch_this_thread() -> bool {
    SLOT.try_with(|slot| {
        if slot.0.get().is_some() {
            return true;
        }
        // SAFETY: gettid has no preconditions.
        let thread = unsafe { libc::gettid() };
        for (index, raised) in RAISED.iter().enumerate() {
            if raised
                .compare_exchange(0, thread, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                PUT[index].store(UNTOUCHED, Ordering::Relaxed);
                slot.0.set(Some(index));
                return true;
            }
        }
        false
    })
    .unwrap_or(false)
}

/// Stops [`put_back`] lowering the calling thread, which stays at normal
/// priority.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub fn forget_this_thread() {
    let _ = SLOT.try_with(|slot| {
        if let Some(index) = slot.0.take() {
            free(index);
        }
    });
}

/// What SIGXCPU's handler did to the calling thread since it last asked:
/// nothing, put it back at normal priority, as the thread that ran too
/// long (`true`) or beside it, or tried and was refused, as Linux kills the
/// program once that thread reaches the hard limit. One atomic load, for a
/// raised thread to ask once a block: the slots share a cache line, which
/// only the handler writes until one has news.
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub fn lowered() -> Option<Result<bool, std::io::Error>> {
    let slot = SLOT.try_with(|slot| slot.0.get()).ok().flatten()?;
    if PUT[slot].load(Ordering::Acquire) == UNTOUCHED {
        return None;
    }
    match PUT[slot].swap(UNTOUCHED, Ordering::AcqRel) {
        UNTOUCHED => None,
        LOWERED => Some(Ok(false)),
        RAN_LONG => Some(Ok(true)),
        error => Some(Err(std::io::Error::from_raw_os_error(error))),
    }
}

/// Has SIGXCPU put every raised thread back at normal priority, rather
/// than end the program, from now on.
fn watch_real_time(soft: libc::rlim_t) {
    static WATCHING: Once = Once::new();
    SOFT_LIMIT.store(soft, Ordering::Relaxed);
    WATCHING.call_once(|| {
        let handler: extern "C" fn(libc::c_int) = put_back;
        // SAFETY: an all-zero sigaction with an empty mask is a valid one,
        // and the handler only makes system calls and touches atomics, as
        // a signal handler may.
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = handler as libc::sighandler_t;
            action.sa_flags = libc::SA_RESTART;
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(libc::SIGXCPU, &action, std::ptr::null_mut()) != 0 {
                tracing::warn!(
                    error = %std::io::Error::last_os_error(),
                    "an audio thread that runs too long would end the program"
                );
            }
        }
    });
}

/// SIGXCPU's handler. Linux sends SIGXCPU to the whole process, not to the
/// thread that ran past the soft RLIMIT_RTTIME (`check_rlimit` in
/// `kernel/time/posix-cpu-timers.c`, with `PIDTYPE_TGID`): that thread
/// usually takes it, but any other may. So it puts every raised thread back
/// at normal priority by its id ([`RAISED`]), and notes for each what came
/// of it ([`PUT`]), for the thread to log outside the handler. Each keeps
/// SCHED_RESET_ON_FORK, which rtkit and [`raise_priority`] set: only a
/// privileged thread may clear it, and asked to, Linux refuses the whole
/// change (EPERM), which leaves the thread to be killed. Once every one is
/// back, the soft limit, which Linux moved a second on, goes back under the
/// hard one for the next. It makes only system calls and touches atomics,
/// as a signal handler may, and leaves errno as it found it.
extern "C" fn put_back(_: libc::c_int) {
    // SAFETY: errno is the calling thread's own; it's given back below.
    let errno = unsafe { *libc::__errno_location() };
    // SAFETY: gettid has no preconditions.
    let this = unsafe { libc::gettid() };
    let watched = RAISED
        .iter()
        .any(|raised| raised.load(Ordering::Acquire) == this);
    let normal = libc::sched_param { sched_priority: 0 };
    let mut all = true;
    for (raised, put) in RAISED.iter().zip(&PUT) {
        let thread = raised.load(Ordering::Acquire);
        if thread == 0 {
            continue;
        }
        // SAFETY: a plain system call, with a value that outlives it, on a
        // thread of this process, which a thread of the same user may lower.
        let lowered = unsafe {
            libc::sched_setscheduler(
                thread,
                libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK,
                &normal,
            )
        };
        let result = if lowered == 0 {
            if thread == this || !watched {
                RAN_LONG
            } else {
                LOWERED
            }
        } else {
            // SAFETY: as above.
            let error = unsafe { *libc::__errno_location() };
            // A thread that has just ended has nothing left to lower.
            if error == libc::ESRCH {
                continue;
            }
            all = false;
            error.max(1)
        };
        put.store(result, Ordering::Release);
    }
    // A thread left at real-time priority is killed at the hard limit
    // whatever the soft one: moved a second on, no more signals come for it
    // meanwhile.
    if all {
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: plain system calls on the process's limits, with values
        // that outlive them.
        unsafe {
            if libc::getrlimit(libc::RLIMIT_RTTIME, &mut limit) == 0 {
                limit.rlim_cur = SOFT_LIMIT.load(Ordering::Relaxed).min(limit.rlim_max);
                libc::setrlimit(libc::RLIMIT_RTTIME, &limit);
            }
        }
    }
    // SAFETY: as above.
    unsafe { *libc::__errno_location() = errno };
}

/// Linux neither throttles a background window's process nor needs a
/// finer timer.
#[cfg(feature = "discord")]
pub fn full_speed(_: bool) {}

pub struct Output(Simple);

impl Output {
    pub fn open(
        speaker: Option<&str>,
        name: &'static str,
        format: Format,
        queue: Option<Duration>,
    ) -> Result<Self> {
        let sink = sink(speaker)?;
        let attributes = queue
            .map(|queue| -> Result<BufferAttr> {
                Ok(BufferAttr {
                    maxlength: u32::MAX,
                    tlength: u32::try_from(format.bytes(queue))?,
                    prebuf: u32::MAX,
                    minreq: u32::MAX,
                    fragsize: u32::MAX,
                })
            })
            .transpose()?;
        open(
            Direction::Playback,
            sink.as_deref(),
            name,
            format,
            attributes.as_ref(),
        )
        .map(Self)
    }

    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.0.write(bytes).map_err(|error| anyhow!("{error}"))
    }

    pub fn drain(&mut self) -> Result<()> {
        self.0.drain().map_err(|error| anyhow!("{error}"))
    }

    pub fn flush(&mut self) {
        let _ = self.0.flush();
    }

    pub fn latency(&self) -> Duration {
        self.0
            .get_latency()
            .map_or(Duration::ZERO, |latency| Duration::from_micros(latency.0))
    }
}

pub struct Input(Simple);

impl Input {
    pub fn open(
        microphone: Option<&str>,
        name: &'static str,
        format: Format,
        fragment: Duration,
    ) -> Result<Self> {
        let source = source(microphone)?;
        let attributes = BufferAttr {
            maxlength: u32::MAX,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: u32::try_from(format.bytes(fragment))?,
        };
        open(
            Direction::Record,
            source.as_deref(),
            name,
            format,
            Some(&attributes),
        )
        .map(Self)
    }

    pub fn read(&mut self, bytes: &mut [u8]) -> Result<()> {
        self.0.read(bytes).map_err(|error| anyhow!("{error}"))
    }
}

fn open(
    direction: Direction,
    device: Option<&str>,
    name: &str,
    format: Format,
    attributes: Option<&BufferAttr>,
) -> Result<Simple> {
    let spec = Spec {
        // The samples go as they are in memory.
        format: if format.float {
            PulseFormat::FLOAT32NE
        } else {
            PulseFormat::S16NE
        },
        channels: u8::try_from(format.channels)?,
        rate: format.rate,
    };
    // The stream's own thread, which libpulse starts here, keeps every core
    // at normal priority rather than wait beside a game's threads on the
    // few Yapfast keeps to (`docs/cpu.md`).
    crate::cpu::keep_free(STREAM_THREAD);
    crate::cpu::unconfined(|| {
        Simple::new(
            None, "Yapfast", direction, device, name, &spec, None, attributes,
        )
    })
    .map_err(|error| anyhow!("{error}"))
}

/// The PulseAudio source behind a WebRTC device: its first entry is the
/// default source, prefixed "default: ", and the rest are the sources that
/// are not monitors, by description.
fn source(microphone: Option<&str>) -> Result<Option<String>> {
    device(microphone, false)
        .map_err(|error| error.context("this microphone is no longer connected"))
}

/// The PulseAudio sink behind a WebRTC playout device, named the same way.
fn sink(speakers: Option<&str>) -> Result<Option<String>> {
    device(speakers, true).map_err(|error| error.context("these speakers are no longer connected"))
}

fn device(id: Option<&str>, sink: bool) -> Result<Option<String>> {
    let Some(description) = id.and_then(|id| id.strip_prefix(NAMED)) else {
        return Ok(None);
    };
    if description.starts_with("default: ") {
        return Ok(None);
    }
    list(sink)?
        .into_iter()
        .find(|found| found.description == description)
        .map(|found| Some(found.name))
        .context("device not found")
}

/// The sink or source a stream for `id` would open now, by its name and
/// index, which a device gets anew each time it comes back: the server moves
/// a stream to another device when its own goes away, and leaves it there
/// when that returns. Empty for the default, which streams follow anyway.
#[cfg(feature = "discord")]
pub fn endpoint(id: Option<&str>, sink: bool) -> String {
    let Some(description) = id
        .and_then(|id| id.strip_prefix(NAMED))
        .filter(|description| !description.starts_with("default: "))
    else {
        return String::new();
    };
    list(sink)
        .ok()
        .and_then(|devices| {
            devices
                .into_iter()
                .find(|found| found.description == description)
        })
        .map(|found| format!("{}#{}", found.name, found.index))
        .unwrap_or_default()
}

/// A sink or source as the server lists it.
struct Listed {
    name: String,
    description: String,
    #[cfg_attr(not(feature = "discord"), allow(dead_code))]
    index: u32,
}

/// Every sink, or every source that is not a monitor.
fn list(sinks: bool) -> Result<Vec<Listed>> {
    let mut mainloop = Mainloop::new().context("no PulseAudio main loop")?;
    let mut context = Context::new(&mainloop, "Yapfast").context("no PulseAudio context")?;
    context
        .connect(None, ContextFlags::NOFLAGS, None)
        .map_err(|error| anyhow!("{error}"))
        .context("could not reach the sound server")?;
    loop {
        iterate(&mut mainloop)?;
        match context.get_state() {
            context::State::Ready => break,
            context::State::Failed | context::State::Terminated => {
                bail!("the sound server refused Yapfast")
            }
            _ => {}
        }
    }
    let found = Rc::new(RefCell::new(Vec::new()));
    let done = Rc::new(RefCell::new(false));
    let introspect = context.introspect();
    // Each listing is its own operation type; keep whichever runs alive.
    let _sinks = sinks.then(|| {
        let found = found.clone();
        let done = done.clone();
        introspect.get_sink_info_list(move |result| match result {
            ListResult::Item(info) => {
                if let (Some(name), Some(description)) = (&info.name, &info.description) {
                    found.borrow_mut().push(Listed {
                        name: name.to_string(),
                        description: description.to_string(),
                        index: info.index,
                    });
                }
            }
            ListResult::End | ListResult::Error => *done.borrow_mut() = true,
        })
    });
    let _sources = (!sinks).then(|| {
        let found = found.clone();
        let done = done.clone();
        introspect.get_source_info_list(move |result| match result {
            ListResult::Item(info) if info.monitor_of_sink.is_none() => {
                if let (Some(name), Some(description)) = (&info.name, &info.description) {
                    found.borrow_mut().push(Listed {
                        name: name.to_string(),
                        description: description.to_string(),
                        index: info.index,
                    });
                }
            }
            ListResult::Item(_) => {}
            ListResult::End | ListResult::Error => *done.borrow_mut() = true,
        })
    });
    while !*done.borrow() {
        iterate(&mut mainloop)?;
    }
    context.disconnect();
    Ok(found.take())
}

fn iterate(mainloop: &mut Mainloop) -> Result<()> {
    match mainloop.iterate(true) {
        IterateResult::Success(_) => Ok(()),
        IterateResult::Quit(_) | IterateResult::Err(_) => {
            bail!("the sound server connection ended")
        }
    }
}

#[cfg(feature = "discord")]
pub use others::others;

/// What other applications play, for Go Live's sound.
#[cfg(feature = "discord")]
mod others {
    use std::{
        cell::RefCell,
        collections::HashMap,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread::JoinHandle,
    };

    use anyhow::{Context as _, Result, anyhow, bail};
    use libpulse_binding::{
        callbacks::ListResult,
        context::{
            self, Context, FlagSet as ContextFlags,
            introspect::SinkInputInfo,
            subscribe::{Facility, InterestMaskSet, Operation},
        },
        def::BufferAttr,
        mainloop::standard::{IterateResult, Mainloop},
        sample::{Format as PulseFormat, Spec},
        stream::{FlagSet as StreamFlags, PeekResult, Stream},
        time::MicroSeconds,
    };

    use super::super::{Hear, Heard, RATE};

    /// Monitors each application's playback stream on its own
    /// (`pa_stream_set_monitor_stream`, as pavucontrol's level meters do).
    pub fn others(
        skip: fn(&str) -> bool,
        heard: Hear,
        stop: Arc<AtomicBool>,
    ) -> Result<JoinHandle<()>> {
        let (ready, started) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("yapfast-stream-capture".into())
            .spawn(move || {
                if let Err(error) = run(skip, heard, &stop, &ready) {
                    let _ = ready.try_send(Err(anyhow!("{error:#}")));
                    tracing::warn!(error = format!("{error:#}"), "stopped capturing sound");
                }
            })
            .context("could not start capturing sound")?;
        started
            .recv()
            .context("sound capture stopped before it started")??;
        Ok(thread)
    }

    /// What PulseAudio's callbacks found, handled after each dispatch so
    /// no callback touches the context.
    enum Change {
        /// A playback stream appeared or changed: look it up.
        Check(u32),
        /// A playback stream, the sink it plays on, and its process.
        Found {
            input: u32,
            sink: u32,
            process: Option<u32>,
            /// An application to leave out.
            skipped: bool,
        },
        /// The monitor source of the sink a stream plays on.
        Monitor {
            input: u32,
            sink: u32,
            process: Option<u32>,
            source: u32,
        },
        Gone(u32),
    }

    type Changes = Rc<RefCell<Vec<Change>>>;
    type Shared = Rc<RefCell<Hear>>;
    type Monitors = HashMap<u32, (u32, Rc<RefCell<Stream>>)>;

    fn run(
        skip: fn(&str) -> bool,
        heard: Hear,
        stop: &AtomicBool,
        ready: &mpsc::SyncSender<Result<()>>,
    ) -> Result<()> {
        let heard: Shared = Rc::new(RefCell::new(heard));
        let mut mainloop = Mainloop::new().context("no PulseAudio main loop")?;
        let context = Rc::new(RefCell::new(
            Context::new(&mainloop, "Yapfast").context("no PulseAudio context")?,
        ));
        context
            .borrow_mut()
            .connect(None, ContextFlags::NOFLAGS, None)
            .context("could not reach PulseAudio")?;
        loop {
            if let IterateResult::Quit(_) | IterateResult::Err(_) = mainloop.iterate(true) {
                bail!("PulseAudio's main loop failed");
            }
            match context.borrow().get_state() {
                context::State::Ready => break,
                context::State::Failed | context::State::Terminated => {
                    bail!("could not reach PulseAudio")
                }
                _ => {}
            }
        }
        let changes = Changes::default();
        let found = changes.clone();
        context.borrow_mut().set_subscribe_callback(Some(Box::new(
            move |facility, operation, index| {
                if facility != Some(Facility::SinkInput) {
                    return;
                }
                found.borrow_mut().push(match operation {
                    Some(Operation::Removed) => Change::Gone(index),
                    _ => Change::Check(index),
                });
            },
        )));
        context
            .borrow_mut()
            .subscribe(InterestMaskSet::SINK_INPUT, |_| {});
        let found = changes.clone();
        context
            .borrow()
            .introspect()
            .get_sink_input_info_list(move |result| {
                if let ListResult::Item(info) = result {
                    found.borrow_mut().push(found_change(info, skip));
                }
            });
        let _ = ready.send(Ok(()));

        // Yapfast's own processes: this worker and the Yapfast that started it.
        let excluded = [std::process::id(), std::os::unix::process::parent_id()];
        let mut streams = Monitors::new();
        while !stop.load(Ordering::Relaxed) {
            mainloop
                .prepare(Some(MicroSeconds(50_000)))
                .map_err(|error| anyhow!("{error}"))?;
            mainloop.poll().map_err(|error| anyhow!("{error}"))?;
            mainloop.dispatch().map_err(|error| anyhow!("{error}"))?;
            if let context::State::Failed | context::State::Terminated =
                context.borrow().get_state()
            {
                bail!("PulseAudio went away");
            }
            let pending = std::mem::take(&mut *changes.borrow_mut());
            for change in pending {
                match change {
                    Change::Check(input) => {
                        let found = changes.clone();
                        context
                            .borrow()
                            .introspect()
                            .get_sink_input_info(input, move |result| {
                                if let ListResult::Item(info) = result {
                                    found.borrow_mut().push(found_change(info, skip));
                                }
                            });
                    }
                    Change::Found {
                        input,
                        sink,
                        process,
                        skipped,
                    } => {
                        if skipped || process.is_some_and(|process| excluded.contains(&process)) {
                            continue;
                        }
                        if streams.get(&input).is_some_and(|(on, _)| *on == sink) {
                            continue;
                        }
                        let found = changes.clone();
                        context
                            .borrow()
                            .introspect()
                            .get_sink_info_by_index(sink, move |result| {
                                if let ListResult::Item(info) = result {
                                    found.borrow_mut().push(Change::Monitor {
                                        input,
                                        sink,
                                        process,
                                        source: info.monitor_source,
                                    });
                                }
                            });
                    }
                    Change::Monitor {
                        input,
                        sink,
                        process,
                        source,
                    } => {
                        forget(&mut streams, &heard, input);
                        match monitor(&mut context.borrow_mut(), &heard, input, process, source) {
                            Ok(stream) => {
                                streams.insert(input, (sink, stream));
                            }
                            Err(error) => {
                                tracing::warn!(
                                    error = format!("{error:#}"),
                                    "could not capture an application's sound"
                                )
                            }
                        }
                    }
                    Change::Gone(input) => forget(&mut streams, &heard, input),
                }
            }
        }
        for input in streams.keys().copied().collect::<Vec<_>>() {
            forget(&mut streams, &heard, input);
        }
        context.borrow_mut().disconnect();
        Ok(())
    }

    fn found_change(info: &SinkInputInfo, skip: fn(&str) -> bool) -> Change {
        Change::Found {
            input: info.index,
            sink: info.sink,
            process: info
                .proplist
                .get_str("application.process.id")
                .and_then(|process| process.parse().ok()),
            skipped: ["application.name", "application.process.binary"]
                .into_iter()
                .filter_map(|key| info.proplist.get_str(key))
                .any(|name| skip(&name)),
        }
    }

    /// Records one playback stream from the monitor source of its sink, as
    /// 48 kHz stereo floats.
    fn monitor(
        context: &mut Context,
        heard: &Shared,
        input: u32,
        process: Option<u32>,
        source: u32,
    ) -> Result<Rc<RefCell<Stream>>> {
        let spec = Spec {
            format: PulseFormat::FLOAT32NE,
            rate: RATE,
            channels: 2,
        };
        let stream = Stream::new(context, "Yapfast stream sound", &spec, None)
            .context("PulseAudio refused a capture stream")?;
        let stream = Rc::new(RefCell::new(stream));
        stream
            .borrow_mut()
            .set_monitor_stream(input)
            .map_err(|error| anyhow!("{error}"))?;
        (heard.borrow_mut())(Heard::Started {
            source: input,
            process,
        });
        let weak = Rc::downgrade(&stream);
        let into = heard.clone();
        // For fragments that don't start on a float's alignment.
        let mut unaligned: Vec<f32> = Vec::new();
        stream
            .borrow_mut()
            .set_read_callback(Some(Box::new(move |_| {
                let Some(stream) = weak.upgrade() else { return };
                let mut stream = stream.borrow_mut();
                while let Ok(peeked) = stream.peek() {
                    match peeked {
                        PeekResult::Data(bytes) => {
                            // SAFETY: every bit pattern is a valid f32.
                            let samples = match unsafe { bytes.align_to::<f32>() } {
                                ([], samples, []) => samples,
                                _ => {
                                    unaligned.clear();
                                    unaligned.extend(bytes.as_chunks::<4>().0.iter().map(
                                        |sample| {
                                            f32::from_ne_bytes([
                                                sample[0], sample[1], sample[2], sample[3],
                                            ])
                                        },
                                    ));
                                    &unaligned
                                }
                            };
                            (into.borrow_mut())(Heard::Samples {
                                source: input,
                                samples,
                            });
                        }
                        PeekResult::Hole(_) => {}
                        PeekResult::Empty => break,
                    }
                    if stream.discard().is_err() {
                        break;
                    }
                }
            })));
        // About 10 ms at a time.
        let attributes = BufferAttr {
            maxlength: u32::MAX,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: RATE / 100 * 2 * 4,
        };
        stream
            .borrow_mut()
            .connect_record(
                Some(&source.to_string()),
                Some(&attributes),
                StreamFlags::ADJUST_LATENCY | StreamFlags::DONT_MOVE,
            )
            .map_err(|error| anyhow!("{error}"))?;
        tracing::debug!(?process, "capturing an application's sound");
        Ok(stream)
    }

    /// Stops recording a playback stream.
    fn forget(streams: &mut Monitors, heard: &Shared, input: u32) {
        if let Some((_, stream)) = streams.remove(&input) {
            let mut stream = stream.borrow_mut();
            stream.set_read_callback(None);
            let _ = stream.disconnect();
        }
        (heard.borrow_mut())(Heard::Stopped { source: input });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, PoisonError};

    use super::*;

    /// Held by the tests that signal SIGXCPU, which puts back every thread
    /// watched, so one's signal doesn't reach the other's thread.
    static SIGNALLING: Mutex<()> = Mutex::new(());

    /// What rtkit leaves a thread it raised with, as far as a process
    /// without privileges can: SCHED_RESET_ON_FORK set, which only a
    /// privileged one may clear, on SCHED_BATCH in place of SCHED_RR, which
    /// it may not take. Whether it took.
    fn as_rtkit_leaves_it() -> bool {
        let param = libc::sched_param { sched_priority: 0 };
        // SAFETY: 0 is the calling thread, and `param` outlives the call.
        unsafe {
            libc::sched_setscheduler(0, libc::SCHED_BATCH | libc::SCHED_RESET_ON_FORK, &param) == 0
        }
    }

    /// SCHED_OTHER with SCHED_RESET_ON_FORK kept, as [`put_back`] leaves it.
    const PUT_BACK_AS: libc::c_int = libc::SCHED_OTHER | libc::SCHED_RESET_ON_FORK;

    /// A thread past the soft RLIMIT_RTTIME is put back at normal priority
    /// on SIGXCPU, which no longer ends the program, and the soft limit
    /// Linux moved on comes back to half the hard one. The thread has
    /// SCHED_RESET_ON_FORK set, as rtkit's have, and the handler must keep
    /// it: asked to clear it, Linux refuses an unprivileged process the
    /// whole change, the thread would stay where it was, and at the hard
    /// limit Linux would kill the program.
    #[test]
    fn a_real_time_thread_that_runs_too_long_goes_back_rather_than_ending_it() {
        let _signalling = SIGNALLING.lock().unwrap_or_else(PoisonError::into_inner);
        limit_real_time(Some(200_000)).expect("RLIMIT_RTTIME lowered");
        let mut limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: `limit` is a whole rlimit for the call to write.
        assert_eq!(
            unsafe { libc::getrlimit(libc::RLIMIT_RTTIME, &mut limit) },
            0
        );
        assert_eq!((limit.rlim_cur, limit.rlim_max), (100_000, 200_000));
        assert!(as_rtkit_leaves_it());
        assert!(watch_this_thread());
        // As Linux does as it signals: the soft limit a second on.
        let moved = libc::rlimit {
            rlim_cur: 1_100_000.min(limit.rlim_max),
            ..limit
        };
        // SAFETY: plain values; `raise` signals this thread, and the handler
        // is set.
        unsafe {
            libc::setrlimit(libc::RLIMIT_RTTIME, &moved);
            libc::raise(libc::SIGXCPU);
            libc::getrlimit(libc::RLIMIT_RTTIME, &mut limit);
        }
        let (put, again) = (lowered(), lowered());
        forget_this_thread();
        assert!(
            matches!(put, Some(Ok(true))),
            "put back, not refused: {put:?}"
        );
        assert!(again.is_none(), "found out once");
        // SAFETY: no preconditions.
        assert_eq!(unsafe { libc::sched_getscheduler(0) }, PUT_BACK_AS);
        assert_eq!(limit.rlim_cur, 100_000, "the soft limit is back");
    }

    /// Whether the calling thread runs at real-time priority, as rtkit or
    /// its own limits left it.
    fn real_time_now() -> bool {
        // SAFETY: no preconditions.
        let policy = unsafe { libc::sched_getscheduler(0) } & !libc::SCHED_RESET_ON_FORK;
        policy == libc::SCHED_RR || policy == libc::SCHED_FIFO
    }

    /// Runs without waiting until the thread is no longer real-time, as a
    /// long job, loading DeepFilterNet once, did on the microphone's thread:
    /// how long that took, or `None` if it still was after `most`.
    fn run_until_put_back(most: Duration) -> Option<Duration> {
        let started = std::time::Instant::now();
        while real_time_now() {
            if started.elapsed() > most {
                return None;
            }
            std::hint::spin_loop();
        }
        Some(started.elapsed())
    }

    /// For real, through rtkit or the user's limits: a raised thread that
    /// runs past the soft limit goes back to normal priority, rather than
    /// have Linux kill the program at the hard one, finds out at its next
    /// block, and rises again; a thread raised beside it is put back too,
    /// and rises again each time. Run long a second time, the thread stays
    /// at normal priority. Run alone, with a log:
    /// `YAPFAST_TEST_AUDIO=1 cargo test -p yapfast-core
    /// a_raised_thread_that_runs_too_long_lives_on -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs YAPFAST_TEST_AUDIO and rtkit, and runs a real-time thread twice 0.1 s"]
    fn a_raised_thread_that_runs_too_long_lives_on() {
        let _signalling = SIGNALLING.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .try_init();
        let (raised, beside_raised) = std::sync::mpsc::channel();
        let (done, beside_done) = std::sync::mpsc::channel::<()>();
        let beside = std::thread::spawn(move || {
            crate::audio_io::raise_thread_priority("beside");
            let _ = raised.send(real_time_now());
            let mut risen = 0;
            while beside_done.try_recv().is_err() {
                std::thread::sleep(Duration::from_millis(1));
                let was = real_time_now();
                crate::audio_io::keep_priority();
                risen += usize::from(!was && real_time_now());
            }
            (risen, real_time_now())
        });
        assert!(
            beside_raised.recv().unwrap(),
            "rtkit raised the thread beside"
        );
        let long = std::thread::spawn(|| {
            crate::audio_io::raise_capture_priority();
            assert!(real_time_now(), "rtkit raised the thread");
            let first = run_until_put_back(Duration::from_millis(400));
            crate::audio_io::keep_priority();
            let again = real_time_now();
            std::thread::sleep(Duration::from_millis(5));
            let second = run_until_put_back(Duration::from_millis(400));
            crate::audio_io::keep_priority();
            (first, again, second, real_time_now())
        });
        let (first, again, second, still) = long.join().expect("the thread ends");
        std::thread::sleep(Duration::from_millis(20));
        let _ = done.send(());
        let (risen, beside_still) = beside.join().expect("the thread beside ends");
        eprintln!(
            "put back after {first:?}, raised again: {again}; put back after {second:?}, \
             real-time after: {still}; the thread beside rose {risen} times, real-time at the \
             end: {beside_still}"
        );
        assert!(first.is_some_and(|first| first < Duration::from_millis(200)));
        assert!(again, "raised again after the first time");
        assert!(second.is_some_and(|second| second < Duration::from_millis(200)));
        assert!(!still, "at normal priority after the second time");
        assert_eq!(risen, 2, "the thread beside rose again each time");
        assert!(beside_still);
    }

    /// Linux signals the whole process, and any of its threads may take
    /// SIGXCPU: the handler puts back the raised thread it doesn't run on
    /// too, by its id. The thread it runs on is the one that ran too long,
    /// as Linux has that one take it first; the others are only beside it,
    /// unless it runs on a thread not watched, which tells it nothing.
    #[test]
    fn sigxcpu_puts_back_a_raised_thread_whichever_thread_takes_it() {
        let _signalling = SIGNALLING.lock().unwrap_or_else(PoisonError::into_inner);
        limit_real_time(Some(200_000)).expect("RLIMIT_RTTIME lowered");
        let (raised, ready) = std::sync::mpsc::channel();
        let (signalled, go_on) = std::sync::mpsc::channel::<()>();
        let other = std::thread::spawn(move || {
            let watched = as_rtkit_leaves_it() && watch_this_thread();
            // SAFETY: gettid has no preconditions.
            let _ = raised.send((watched, unsafe { libc::gettid() }));
            let found: Vec<_> = (go_on.iter())
                .map(|()| lowered().map(|put| put.map_err(|error| error.to_string())))
                .collect();
            forget_this_thread();
            found
        });
        let (watched, thread) = ready.recv().expect("the other thread is raised");
        assert!(watched);
        // SAFETY: `raise` signals this thread, not watched, and the handler
        // is set.
        unsafe { libc::raise(libc::SIGXCPU) };
        // SAFETY: no preconditions; `thread` waits for `go_on`.
        let policy = unsafe { libc::sched_getscheduler(thread) };
        let _ = signalled.send(());
        // Now from a thread watched too, which ran too long.
        assert!(as_rtkit_leaves_it() && watch_this_thread());
        // SAFETY: as above.
        unsafe { libc::raise(libc::SIGXCPU) };
        let this = lowered();
        forget_this_thread();
        let _ = signalled.send(());
        drop(signalled);
        let found = other.join().expect("the other thread ends");
        assert_eq!(policy, PUT_BACK_AS, "the other thread is put back");
        assert_eq!(
            found,
            [Some(Ok(true)), Some(Ok(false))],
            "and it finds out, beside the one that ran long the second time"
        );
        assert!(matches!(this, Some(Ok(true))), "{this:?}");
    }
}
