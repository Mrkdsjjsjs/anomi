# Processor cores

Yapfast keeps its threads to some of the processor's cores, so a game
running beside it has the rest to itself. Settings → Advanced →
Optimization → "CPU load distribution" offers Auto, All cores, and Choose
cores, ticked by physical core and grouped by chiplet and by performance
and efficiency cores. `layout.json` keeps the mode, not the cores, as `cpu_mode`: `"auto"`
(the default), `"all"`, or `{"cores": [8, 9, …]}`, physical cores numbered
from 0. The cores are worked out again at each start, so a new processor,
or the folder copied to another computer, gets cores that fit it. The
setting's line says what was found and which cores are used, such as
"Chiplets: 2, cores: 16, threads: 32. Runs on cores 8-15 (CPUs 8-15,
24-31): the chiplet with the smaller cache, 32 MB."

The code is `crates/yapfast-core/src/cpu.rs`, with `cpu/linux.rs` and
`cpu/windows.rs`: `detect` reads the layout, `choose` picks the cores for
a mode, `apply` sets them, and `describe` writes the line.

## What Auto takes

In order:

1. Nothing changes where someone else started Yapfast on fewer CPUs than
   are online, such as `taskset`, a cgroup's cpuset, a sandbox or systemd's
   `AllowedCPUs`, nor on a processor too small to split: 2 or 3 cores, or 4
   without SMT.
2. A processor with performance and efficiency cores, such as Intel's
   12th generation and later: its efficiency cores.
3. A processor with several L3 caches, a chiplet each on AMD's: one without
   core 0; of those, the one with the smallest L3; then the one whose best
   core ranks lowest; then the last. A chiplet of 6 cores or more, as on
   every Zen 3 and later, is taken whole; a smaller one, such as a Zen 2
   CCX, gives its last cores as below.
4. One L3: the last cores, never core 0: 1 of 4, 2 of 6 or 8, 3 of 12, 4 of
   16, a quarter from there. The last are the lowest ranked, then the
   highest numbered.

A core always comes with its SMT sibling, so a game's thread doesn't share
a core with Yapfast's. Ranks are amd-pstate's preferred-core ranking, else
ACPI CPPC's highest performance, else the highest clock; Windows gives
none.

| Processor | Auto takes |
|---|---|
| Ryzen 9 9950X3D (this machine) | cores 8-15, CPUs 8-15 and 24-31: the 32 MB chiplet; the 96 MB V-cache one stays to the game |
| Ryzen 9 9950X | cores 8-15, the chiplet without core 0 |
| Ryzen 9 7900X3D | cores 6-11, the 32 MB chiplet |
| Core i9-13900K | its 16 efficiency cores, CPUs 16-31 |
| Ryzen AI 9 HX 370 (Strix Point) | its 8 Zen 5c cores, the smaller L3 |
| Ryzen 7 5800X | cores 6 and 7 |
| Ryzen 5 3600 | cores 4 and 5, of the second CCX |
| 4 cores, 8 threads | core 3 |
| 4 cores without SMT, 2 cores | every core |

On an X3D processor under Linux, the kernel's `amd_x3d_vcache` driver
ranks the chiplets by `amd_x3d_mode`. In `frequency` mode, this machine's
as found, it prefers the 32 MB chiplet for the busiest threads, so a
lightly threaded game starts on the cores Auto takes. Auto keeps to the
smaller L3 whatever the mode: it matches what Windows does with X3D, and
keeps Yapfast out of the cache games gain from. For the game to go to the
V-cache, set the system's mode, as root, which Yapfast never does itself:

    echo cache > /sys/bus/platform/drivers/amd_x3d_vcache/AMDI0101:00/amd_x3d_mode

## How the cores are set

**Linux** keeps a mask for each thread. `apply` walks `/proc/self/task`
and sets every thread's (`sched_setaffinity`), passing again for threads
started meanwhile, at most three times: about 100 calls, under a
millisecond. A thread started later takes the mask of the thread that
starts it, and so does a program. Anomi applies the mode first thing as it
starts, before its other threads, and again as Settings changes it.

The mask the process started with is read once, before Yapfast narrows it:
All goes back to it, and Auto compares it with the CPUs online. A process
Yapfast starts, such as a stream worker, would start on the narrowed mask,
so the first process hands its own down in `YAPFAST_STARTED_ON_CPUS`, set
while it still has one thread, and a process takes it only from a parent
that is the same program.

**Windows** sets the process's default CPU sets
(`SetProcessDefaultCpuSets`), leaving out those another process holds,
such as a game in Game Mode. CPU sets are soft, and stay out of the way of
core parking. Where Windows refuses them, the process's affinity mask,
within its processor group, takes their place. None of it has run on
Windows yet: whether threads already running move at once, whether AMD's
driver parking the frequency chiplet during a game moves Yapfast onto the
game's, and how efficiency cores meet EcoQoS are open.

Reading the layout takes 1-2 ms, at start and as the mode or the settings
page changes. Afterwards nothing runs: the system keeps the masks.

## What keeps every core

- **A call's audio threads** (`yapfast-discord-call`, `-in`, `-out`), and
  the microphone test's (`yapfast-gate-meter` once it plays back, and
  `yapfast-mic-test`), raise their priority as they start
  (`audio_io::raise_thread_priority`). On Linux they ask for `SCHED_FIFO`
  themselves where the user's limits allow real-time priority (`ulimit
  -r`), and else ask rtkit, on the system bus, for `SCHED_RR` at the same
  priority: 10 for the call's and the speakers', 9 for a microphone's.
  Either way the policy carries `SCHED_RESET_ON_FORK`, so a thread one of
  them starts begins at normal priority. At real-time priority they stay
  on Yapfast's cores, where they run ahead of any game thread. A thread
  whose priority couldn't be raised, or that `YAPFAST_VOICE_NO_PRIORITY`
  keeps at normal priority, runs on every core instead, so it needn't wait
  beside a game's threads.

  rtkit raises only threads of a process whose `RLIMIT_RTTIME` is within
  its own limit, 200 ms by default, so Yapfast lowers it to that first:
  the processor time a real-time thread may take without sleeping, which
  Linux counts in scheduler ticks, a millisecond at 1000 Hz, and starts
  again each time the thread wakes at real-time priority from a sleep. At the hard limit Linux kills the
  whole process with SIGKILL. So a raised thread makes nothing slow
  itself: a suppressor chosen in a call or the microphone test loads on a
  thread of its own (`yapfast-denoise`), as does WebRTC's echo
  cancellation switched (`yapfast-echo`), while the one before goes on,
  and each capture's WebRTC modules are made by the thread that asks for
  the microphone (`crate::rebuild`, `discord_audio.rs`). The soft limit
  goes to half the hard one, 100 ms, where Linux sends SIGXCPU, and moves
  it a second on, so it comes once: Yapfast's handler then puts every
  raised thread back at normal priority, by its id, as the signal goes to
  the whole process and any of its threads may take it, and sets the soft
  limit back (`audio_io/pulse.rs`). That leaves the thread that ran long
  100 ms of its own time before the kill, and at normal priority Linux
  stops counting. The handler keeps `SCHED_RESET_ON_FORK`: only a
  privileged process may clear it, and asked to, Linux refuses the whole
  change. Each thread finds out at its next block, logs it, and asks to be
  raised again, its slow job over. The handler runs on the thread that ran
  long, as Linux has that one take the signal first, and only that thread
  counts it: put back a second time for its own long job, it stays at
  normal priority, on every core, until it ends, while the threads put
  back beside it rise again each time. A thread ending, as a call's
  microphone does, goes back to normal priority first, so what it frees
  then, a suppressor's model, is freed there. The same handler watches
  the threads that raise themselves where the user's own limits set
  `RLIMIT_RTTIME`. A long job on an audio thread stutters rather than ends
  the call.
- **libpulse's stream threads** (`threaded-ml`, one per stream) carry the
  samples to the sound server at normal priority, and run on every core.
- **A browser** that opening a link starts runs on every core: `xdg-open`
  starts on the mask Yapfast started with.

A stream worker doesn't: it runs on the cores Settings chose, in every
mode and wherever its stream encodes, x264 on the processor too. It's told
them as it starts, all of them included, rather than left to inherit them,
as Windows wouldn't pass them on, and again at once as Settings changes
them while it runs. Where x264 can't keep up on them, the dock says so
once in the stream; see [Go Live](discord/go-live.md#encoding-on-the-gpu).

## Thread pools

Video decoders, for streams and files, and the x264 and libvpx encoders
take one thread for each logical CPU Yapfast runs on, at most 4
(`cpu::threads`), so a stream's x264 slices its frames for the cores
chosen, not the machine's: 2 threads on one core with its sibling, 4 on
two cores or more. `cpu::apply` counts the CPUs as it sets them, the
mask's, or for all of them the system's count then, as on Windows the
standard library's count doesn't see CPU sets. A stream worker reads that
count for each frame it encodes, and opens x264 again, with a keyframe,
when a change of cores gives another. openmls, which DAVE's encryption
uses, runs rayon's global pool, which would start one idle thread for each
logical CPU, 32 here; `cpu::init_pools` builds it at start with 2, named
`yapfast-mls-0` and `-1`.

## Seeing it

With Anomi running, on Linux (its stream workers are `anomi` too, started
after it, so `pgrep -o` names Anomi itself):

    pid=$(pgrep -xo anomi)
    grep -H Cpus_allowed_list /proc/$pid/task/*/status
    ps -L -o tid,cls,rtprio,psr,comm -p $pid

On this machine with Auto, every thread shows `8-15,24-31` but the
`threaded-ml` threads and any audio thread left at normal priority, which
show `0-31`. In a call, the three `yapfast-discord` audio threads show
class `RR` (`FF` where the limits allow it) at priority 10 or 9, the rest
`TS`. The log says `Yapfast runs on these logical CPUs` with the list, and
for each audio thread how its priority was raised, such as `SCHED_RR 10
through rtkit`; each thread one SIGXCPU put back says "an audio thread
went back to normal priority, as it or another ran too long without
waiting, and raised it again", with `thread` and `ran_long`, and a
handler that couldn't put one back logs an error. A
stream worker (`pgrep -f -- --stream-worker`) shows its
own masks the same way: the cores chosen on every thread, x264's too.

## What it can't promise

- Yapfast takes itself off the game's cores, nothing more: a game, and
  Wine or Proton, usually spreads its threads over every CPU, ours too.
  Yapfast never sets another program's cores.
- A game's thread on the SMT sibling of a core Yapfast uses still shares
  that core with it.
- Efficiency cores on a laptop on battery run the noise suppressor and x264
  slower; that hasn't been measured.
- A stream x264 encodes gets no more cores than those chosen: on one or
  two, a large screen at 60 fps can be more than it encodes in time, and
  the stream then sends fewer frames than it asked for until its quality,
  its frame rate or the cores change, or a graphics card encodes it.
