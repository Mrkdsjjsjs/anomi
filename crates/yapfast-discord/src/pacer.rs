//! Keeps a Go Live stream under its ceiling on the wire. The ceiling is the
//! bitrate limit the person chose, or Discord's cap for the stream
//! (`stream_connection::cap`), and it counts everything the stream
//! connection sends: the video's packets and their resends, the stream's
//! sound and RTCP, each with its RTP header and extensions, the transport's
//! tag and counter, and the IPv4 and UDP headers before it, as a server
//! policing the stream would count them.
//!
//! Every packet goes through one token bucket ([`Bucket`]), which refills
//! at [`rate`] and holds at most [`depth`], 10 ms at the ceiling: over any
//! span of time the stream sends at most the depth plus the rate for that
//! long: 97% of the ceiling over any second, and 106% of it over any 100 ms
//! (more below 2.4 Mbit/s, where the depth is two datagrams). Video waits
//! for the bucket; resends wait for it too, and for a
//! share of their own ([`RESEND_SHARE`]), so a burst of NACKs can't starve
//! the frames; the sound and RTCP go at once and are counted, taking the
//! bucket a packet or two below empty.
//!
//! The encoder aims under what that leaves the video ([`budget`]): the
//! bucket's rate less [`HEADROOM`], less the sound, resends and RTCP sent
//! lately, less each packet's bytes past its share of the frame, as
//! measured ([`Limiter::budget`]). What it sends past its target waits in
//! the queue, its rate control buffer at most (`native/h264.c`: a fifth of
//! a second past Discord's documented 9 Mbit/s, half a second otherwise,
//! `stream_connection::encoder_buffer`); past [`overshoot`] its target
//! drops by the overshoot at once ([`Limiter::trim`]), and a frame that
//! would wait past [`most_queued`] is dropped whole, with those after it
//! until a keyframe, never sent late in part ([`Limiter::admit`]).

use std::time::{Duration, Instant};

use crate::rtp;

/// The share of the ceiling, in hundredths, that the bucket's rate stays
/// under besides its depth: room for a receiver's timing, which may count
/// a few milliseconds' packets in the next second.
const SAFETY: u64 = 3;
/// The most the bucket lets out at once: this long at the ceiling.
pub const DEPTH: Duration = Duration::from_millis(10);
/// And at least two whole datagrams, in bits, so a packet always fits.
const LEAST_DEPTH: u64 = 2 * 1500 * 8;
/// Resends take at most this share of the bucket's rate, in hundredths,
/// and hold at most this share of its depth, but a datagram.
pub const RESEND_SHARE: u64 = 20;
const LEAST_RESEND_DEPTH: u64 = 1500 * 8;
/// The share of the bucket's rate, in hundredths, the encoder leaves free:
/// the queue drains after a keyframe even while the measured overhead
/// moves.
const HEADROOM: u64 = 4;
/// IPv4's and UDP's headers before each datagram; IPv6's take 20 more.
pub const DATAGRAM: usize = 28;
pub const DATAGRAM_V6: usize = DATAGRAM + 20;
/// A video packet's bytes besides its share of the frame: its header and
/// extensions, the transport's tag and counter, and IPv4 and UDP's headers.
const PACKET_OVERHEAD: usize = rtp::PREFIX + rtp::SEALING + DATAGRAM;
/// What each video packet adds to its share of the frame before any was
/// measured: [`PACKET_OVERHEAD`], H264's fragmentation header and a share of
/// DAVE's trailer.
const ASSUMED_PER_PACKET: u64 = PACKET_OVERHEAD as u64 + 12;
/// The most a measurement may say it adds.
const MOST_PER_PACKET: u64 = 400;
/// The stream's sound, in bit/s on the wire, counted whether it plays or
/// not, since it may start at any moment: Opus at 64 kbit/s in 50 packets
/// a second, each with its RTP header, the transport's tag and counter, and
/// IPv4 and UDP's headers.
pub const SOUND: u64 = 64_000 + 50 * (12 + rtp::SEALING + DATAGRAM) as u64 * 8;
/// What's queued may take this much longer than the encoder's rate
/// control buffer to go out at the bucket's rate before the encoder has
/// overshot ([`overshoot`]).
const SLACK: Duration = Duration::from_millis(50);

/// What's queued may take this long to go out at the bucket's rate before
/// the encoder, with a rate control buffer of `buffer`, has overshot: a
/// keyframe or a scene cut that keeps to the buffer takes a little less.
/// Past it the encoder's target drops, and the stream worker skips encoding
/// frames until the queue is shorter again.
pub fn overshoot(buffer: Duration) -> Duration {
    buffer + SLACK
}

/// A frame that would wait longer than this behind those queued is dropped
/// whole, and the frames after it until a keyframe: what [`overshoot`]
/// lets queue and the buffer besides, so a frame encoded while the queue
/// was within it always fits. One with nothing before it goes, however
/// large, so a keyframe always gets through.
pub fn most_queued(buffer: Duration) -> Duration {
    overshoot(buffer) + buffer
}
/// The encoder's target, in thousandths of what it would be, goes no lower
/// for overshooting...
const LEAST_TRIM: u64 = 500;
/// ...drops at most this often...
const TRIM_EVERY: Duration = Duration::from_millis(300);
/// ...and, once the queue stayed short this long after the last drop,
/// comes back by [`EASE`] thousandths each [`EASE_EVERY`].
const EASE_AFTER: Duration = Duration::from_secs(1);
const EASE_EVERY: Duration = Duration::from_millis(500);
const EASE: u64 = 50;

/// What the [`Meter`] counts in each slot of time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Bytes on the wire: video packets as first sent, resends, the
    /// stream's sound, and RTCP.
    Video,
    Resend,
    Sound,
    Report,
    /// The encoder's frames, before DAVE and packets.
    Encoded,
    /// Those frames' packets on the wire, counted as they're queued, and
    /// how many there were.
    Packetized,
    Packets,
}

const KINDS: usize = 7;
/// What goes on the wire.
const ON_WIRE: [Kind; 4] = [Kind::Video, Kind::Resend, Kind::Sound, Kind::Report];

/// The bits the bucket holds at most at `ceiling` bit/s: [`DEPTH`] of it,
/// and at least two whole datagrams.
pub fn depth(ceiling: u64) -> u64 {
    (ceiling * DEPTH.as_millis() as u64 / 1000).max(LEAST_DEPTH)
}

/// The bit/s the bucket refills at under `ceiling`: so that a second,
/// with the bucket full as it starts, sends at most 97% of the ceiling.
pub fn rate(ceiling: u64) -> u64 {
    (ceiling * (100 - SAFETY) / 100)
        .saturating_sub(depth(ceiling))
        .max(ceiling / 2)
        .max(1)
}

/// A frame of `payload` bytes as its packets go on the wire, in bytes, as
/// an estimate before it's cut into packets: a little over, as each of its
/// NAL units or OBUs may start a packet.
pub fn frame_on_wire(payload: usize) -> u64 {
    let packets = payload / rtp::CHUNK + 6;
    (payload + packets * (PACKET_OVERHEAD + 2)) as u64
}

/// The bit/s the encoder may aim for under `ceiling`, at `fps` frames a
/// second, when each video packet adds `per_packet` bytes to its share of
/// the frame and the sound, resends and RTCP take `other` bit/s: the
/// bucket's rate less [`HEADROOM`] and those, with each frame's packets'
/// overhead on top of its bytes. The stream's sound counts at least as
/// [`SOUND`].
pub fn budget(ceiling: u64, fps: u32, per_packet: u64, other: u64) -> u64 {
    let free = rate(ceiling) * (100 - HEADROOM) / 100;
    // The last packet of each frame is part full: a packet's overhead for
    // each frame, besides the share of its bytes.
    let frames = u64::from(fps.max(1)) * per_packet * 8;
    let left = free.saturating_sub(other.max(SOUND)).saturating_sub(frames);
    let chunk = rtp::CHUNK as u64;
    left * chunk / (chunk + per_packet)
}

/// The bit/s the encoder may aim for under `ceiling` before anything was
/// measured: [`budget`] with the overhead assumed, and the sound's.
pub fn encoder_ceiling(ceiling: u64, fps: u32) -> u64 {
    budget(ceiling, fps, ASSUMED_PER_PACKET, SOUND)
}

/// The least ceiling under which the encoder may aim for `video` bit/s at
/// `fps` frames a second ([`encoder_ceiling`]): the wire that video takes,
/// with its packets' overhead, the sound and the limiter's margins, when
/// the video's rate is the cap, as Discord's own is.
pub fn wire_for(video: u64, fps: u32) -> u64 {
    let chunk = rtp::CHUNK as u64;
    let needed = video * (chunk + ASSUMED_PER_PACKET) / chunk
        + SOUND
        + u64::from(fps.max(1)) * ASSUMED_PER_PACKET * 8;
    // Past 2.4 Mbit/s the bucket's rate is 96% of the ceiling, and the
    // encoder leaves 4% of that free; below, the depth's floor takes more.
    let mut ceiling = needed * 10_000 / 9_216;
    while encoder_ceiling(ceiling, fps) < video {
        ceiling += ceiling / 1000 + 1;
    }
    ceiling
}

/// Bits times 10⁹: a bucket refilling at its rate for a nanosecond adds a
/// whole number of them.
const NANOS: i128 = 1_000_000_000;

/// A token bucket: what may be sent, refilling at `rate` bit/s up to its
/// depth.
#[derive(Clone, Debug)]
pub struct Bucket {
    rate: u64,
    /// Its depth and what it holds, in [`NANOS`].
    depth: i128,
    tokens: i128,
    at: Instant,
}

impl Bucket {
    /// A full bucket, as a stream starts.
    pub fn new(rate: u64, depth: u64, now: Instant) -> Self {
        let depth = i128::from(depth) * NANOS;
        Self {
            rate: rate.max(1),
            depth,
            tokens: depth,
            at: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        if now <= self.at {
            return;
        }
        // A second refills any bucket here, whose depth is far less.
        let elapsed = now.duration_since(self.at).as_nanos().min(NANOS as u128) as i128;
        self.tokens = (self.tokens + i128::from(self.rate) * elapsed).min(self.depth);
        self.at = now;
    }

    /// What `bits` take of it: a packet larger than the bucket takes it
    /// whole, so it still goes, once.
    fn need(&self, bits: u64) -> i128 {
        (i128::from(bits) * NANOS).min(self.depth)
    }

    /// Whether it holds `bits` at `now`.
    pub fn has(&mut self, now: Instant, bits: u64) -> bool {
        self.refill(now);
        self.tokens >= self.need(bits)
    }

    /// Takes `bits` if it holds them.
    pub fn take(&mut self, now: Instant, bits: u64) -> bool {
        if !self.has(now, bits) {
            return false;
        }
        self.tokens -= i128::from(bits) * NANOS;
        true
    }

    /// Takes `bits` whether it holds them or not, as for a packet that
    /// mustn't wait: what follows waits the longer.
    pub fn force(&mut self, now: Instant, bits: u64) {
        self.refill(now);
        self.tokens -= i128::from(bits) * NANOS;
    }

    /// How long until it holds `bits`.
    pub fn wait(&mut self, now: Instant, bits: u64) -> Duration {
        self.refill(now);
        let short = self.need(bits) - self.tokens;
        if short <= 0 {
            return Duration::ZERO;
        }
        let nanos = short / i128::from(self.rate) + 1;
        Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
    }

    /// Refills at `rate` up to `depth` bits from now on: what it holds stays,
    /// at most the new depth, so a lower ceiling holds at once.
    pub fn set(&mut self, now: Instant, rate: u64, depth: u64) {
        self.refill(now);
        self.rate = rate.max(1);
        self.depth = i128::from(depth) * NANOS;
        self.tokens = self.tokens.min(self.depth);
    }

    pub fn rate(&self) -> u64 {
        self.rate
    }

    /// Its depth in bits.
    pub fn depth(&self) -> u64 {
        (self.depth / NANOS) as u64
    }
}

/// How long a slot of the [`Meter`] is.
pub const SLOT: Duration = Duration::from_millis(100);
/// The slots it keeps: the five seconds its peaks are over, and the second
/// before the first of them.
const SLOTS: usize = 64;
/// Slots in a second, and in the five seconds the peaks are over.
const SECOND: u64 = 10;
const PEAKS: u64 = 50;
/// Slots the overhead is measured over: two seconds.
const MEASURED: u64 = 20;
/// The encoder's bytes in them below which the measurement isn't trusted.
const LEAST_MEASURED: u64 = 16_000;

#[derive(Clone, Copy)]
struct Slot {
    index: u64,
    counts: [u64; KINDS],
}

const EMPTY: Slot = Slot {
    index: u64::MAX,
    counts: [0; KINDS],
};

/// Counts what the stream sends by [`Kind`], in 100 ms slots, in a ring
/// that never grows.
pub struct Meter {
    started: Instant,
    slots: [Slot; SLOTS],
}

/// What went on the wire, in bit/s: over the last whole 100 ms and the last
/// whole second, and the most any 100 ms and any second (counted in whole
/// slots) of the last five carried.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rates {
    pub last_100ms: u64,
    pub last_second: u64,
    pub peak_100ms: u64,
    pub peak_second: u64,
}

impl Meter {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            slots: [EMPTY; SLOTS],
        }
    }

    fn index(&self, now: Instant) -> u64 {
        (now.saturating_duration_since(self.started).as_millis() / SLOT.as_millis()) as u64
    }

    pub fn add(&mut self, now: Instant, kind: Kind, count: u64) {
        let index = self.index(now);
        let slot = &mut self.slots[(index % SLOTS as u64) as usize];
        if slot.index != index {
            *slot = Slot {
                index,
                counts: [0; KINDS],
            };
        }
        slot.counts[kind as usize] += count;
    }

    fn count(&self, index: u64, kind: Kind) -> u64 {
        let slot = &self.slots[(index % SLOTS as u64) as usize];
        if slot.index == index {
            slot.counts[kind as usize]
        } else {
            0
        }
    }

    fn wire(&self, index: u64) -> u64 {
        ON_WIRE.iter().map(|&kind| self.count(index, kind)).sum()
    }

    /// `kind` over the `slots` whole slots before `now`.
    pub fn recent(&self, now: Instant, kind: Kind, slots: u64) -> u64 {
        let current = self.index(now);
        (current.saturating_sub(slots)..current)
            .map(|index| self.count(index, kind))
            .sum()
    }

    pub fn rates(&self, now: Instant) -> Rates {
        let current = self.index(now);
        let second = |end: u64| -> u64 {
            (end.saturating_sub(SECOND - 1)..=end)
                .map(|index| self.wire(index))
                .sum()
        };
        let (mut peak_slot, mut peak_second) = (0, 0);
        for end in current.saturating_sub(PEAKS)..current {
            peak_slot = peak_slot.max(self.wire(end));
            peak_second = peak_second.max(second(end));
        }
        let last = current.checked_sub(1);
        Rates {
            last_100ms: last.map_or(0, |last| self.wire(last)) * 8 * SECOND,
            last_second: last.map_or(0, second) * 8,
            peak_100ms: peak_slot * 8 * SECOND,
            peak_second: peak_second * 8,
        }
    }
}

/// What [`Limiter::admit`] decided for a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admitted {
    /// It goes in the queue; else it's dropped whole.
    pub send: bool,
    /// The encoder's target dropped: it overshot.
    pub trimmed: bool,
}

/// A stream's sends under its ceiling: the bucket, the resends' share of
/// it, what went out, and how far the encoder's target is lowered for
/// overshooting.
pub struct Limiter {
    ceiling: u64,
    /// The encoder's rate control buffer, which [`overshoot`] and
    /// [`most_queued`] follow.
    buffer: Duration,
    /// The bytes of IP and UDP headers before each datagram.
    datagram: usize,
    bucket: Bucket,
    resends: Bucket,
    meter: Meter,
    /// The encoder's target, in thousandths of what it would be.
    trim: u64,
    trimmed: Option<Instant>,
    eased: Option<Instant>,
}

fn resend_rate(ceiling: u64) -> u64 {
    rate(ceiling) * RESEND_SHARE / 100
}

fn resend_depth(ceiling: u64) -> u64 {
    (depth(ceiling) * RESEND_SHARE / 100).max(LEAST_RESEND_DEPTH)
}

impl Limiter {
    /// Sends under `ceiling` bit/s on the wire from `now`, from an encoder
    /// with a rate control buffer of `buffer`, over IPv4.
    pub fn new(ceiling: u64, buffer: Duration, now: Instant) -> Self {
        Self {
            ceiling,
            buffer,
            datagram: DATAGRAM,
            bucket: Bucket::new(rate(ceiling), depth(ceiling), now),
            resends: Bucket::new(resend_rate(ceiling), resend_depth(ceiling), now),
            meter: Meter::new(now),
            trim: 1000,
            trimmed: None,
            eased: None,
        }
    }

    pub fn ceiling(&self) -> u64 {
        self.ceiling
    }

    /// The bucket's rate, in bit/s.
    pub fn rate(&self) -> u64 {
        self.bucket.rate()
    }

    /// The bucket's depth, in bits.
    pub fn depth(&self) -> u64 {
        self.bucket.depth()
    }

    /// Counts `bytes` of IP and UDP headers before each datagram from now
    /// on: [`DATAGRAM_V6`] over IPv6.
    pub fn set_datagram(&mut self, bytes: usize) {
        self.datagram = bytes;
    }

    /// A datagram of `sealed` bytes as it goes on the wire, in bytes: with
    /// the IP and UDP headers before it.
    pub fn on_wire(&self, sealed: usize) -> u64 {
        (sealed + self.datagram) as u64
    }

    /// How long what's queued may take to go out before the encoder has
    /// overshot ([`overshoot`]).
    pub fn overshoot(&self) -> Duration {
        overshoot(self.buffer)
    }

    /// Holds the stream under `ceiling` from `now` on, from an encoder with
    /// a rate control buffer of `buffer`: a lower one at once.
    pub fn set_ceiling(&mut self, now: Instant, ceiling: u64, buffer: Duration) {
        self.ceiling = ceiling;
        self.buffer = buffer;
        self.bucket.set(now, rate(ceiling), depth(ceiling));
        self.resends
            .set(now, resend_rate(ceiling), resend_depth(ceiling));
    }

    /// Takes a video packet of `bytes` on the wire, if the bucket holds it.
    pub fn take_video(&mut self, now: Instant, bytes: u64) -> bool {
        let taken = self.bucket.take(now, bytes * 8);
        if taken {
            self.meter.add(now, Kind::Video, bytes);
        }
        taken
    }

    /// Takes a resend of `bytes` on the wire, if both the bucket and the
    /// resends' share hold it.
    pub fn take_resend(&mut self, now: Instant, bytes: u64) -> bool {
        let bits = bytes * 8;
        if !self.resends.has(now, bits) || !self.bucket.has(now, bits) {
            return false;
        }
        self.resends.take(now, bits);
        self.bucket.take(now, bits);
        self.meter.add(now, Kind::Resend, bytes);
        true
    }

    /// Counts a packet of `bytes` on the wire that went at once: the
    /// stream's sound or RTCP.
    pub fn force(&mut self, now: Instant, kind: Kind, bytes: u64) {
        self.bucket.force(now, bytes * 8);
        self.meter.add(now, kind, bytes);
    }

    /// How long until the bucket holds `bits` of video.
    pub fn video_wait(&mut self, now: Instant, bits: u64) -> Duration {
        self.bucket.wait(now, bits)
    }

    /// How long until a resend of `bits` may go.
    pub fn resend_wait(&mut self, now: Instant, bits: u64) -> Duration {
        self.bucket
            .wait(now, bits)
            .max(self.resends.wait(now, bits))
    }

    /// Counts a frame queued: `encoded` bytes from the encoder, which went
    /// into `packets` packets of `wire` bytes on the wire.
    pub fn queued_frame(&mut self, now: Instant, encoded: usize, wire: u64, packets: usize) {
        self.meter.add(now, Kind::Encoded, encoded as u64);
        self.meter.add(now, Kind::Packetized, wire);
        self.meter.add(now, Kind::Packets, packets as u64);
    }

    /// How long `queued` bits take to go out at the bucket's rate.
    pub fn queued_for(&self, queued: u64) -> Duration {
        Duration::from_micros(queued.saturating_mul(1_000_000) / self.rate())
    }

    /// Whether a frame of `frame` bits on the wire goes in the queue behind
    /// `queued` bits: not when it would wait past [`most_queued`] behind
    /// others. Waiting past [`overshoot`], or dropped, it lowers the
    /// encoder's target ([`Self::trim`]) by as much as it overshot.
    pub fn admit(&mut self, now: Instant, queued: u64, frame: u64) -> Admitted {
        let wait = self.queued_for(queued.saturating_add(frame));
        let send = queued == 0 || wait <= most_queued(self.buffer);
        let trimmed = wait > self.overshoot() && self.lower(now, wait);
        Admitted { send, trimmed }
    }

    /// Lowers the encoder's target by the share `wait` overshot
    /// [`overshoot`], at most every [`TRIM_EVERY`]; whether it did.
    fn lower(&mut self, now: Instant, wait: Duration) -> bool {
        if self
            .trimmed
            .is_some_and(|at| now.saturating_duration_since(at) < TRIM_EVERY)
        {
            return false;
        }
        let factor = (self.overshoot().as_micros() * 1000 / wait.as_micros().max(1)) as u64;
        let trim = (self.trim * factor.min(1000) / 1000).max(LEAST_TRIM);
        self.trimmed = Some(now);
        let lowered = trim < self.trim;
        self.trim = trim;
        lowered
    }

    /// With `queued` bits queued at `now`: once the queue stayed short a
    /// while after the last overshoot, the encoder's target comes back a
    /// step; whether it did.
    pub fn settle(&mut self, now: Instant, queued: u64) -> bool {
        if self.trim >= 1000 || self.queued_for(queued) > self.overshoot() / 2 {
            return false;
        }
        let since = |at: Option<Instant>, long: Duration| {
            at.is_none_or(|at| now.saturating_duration_since(at) >= long)
        };
        if !since(self.trimmed, EASE_AFTER) || !since(self.eased, EASE_EVERY) {
            return false;
        }
        self.trim = (self.trim + EASE).min(1000);
        self.eased = Some(now);
        true
    }

    /// The encoder's target, in thousandths of what it would be: lower for
    /// a while after it overshot what the bucket lets out.
    pub fn trim(&self) -> u64 {
        self.trim
    }

    /// What each video packet added to its share of the frame over the
    /// last two seconds, or what it's assumed to add before there's enough
    /// to measure.
    pub fn per_packet(&self, now: Instant) -> u64 {
        let measured = |kind| self.meter.recent(now, kind, MEASURED);
        let (encoded, wire, packets) = (
            measured(Kind::Encoded),
            measured(Kind::Packetized),
            measured(Kind::Packets),
        );
        if encoded < LEAST_MEASURED || packets == 0 {
            return ASSUMED_PER_PACKET;
        }
        (wire.saturating_sub(encoded) / packets).clamp(PACKET_OVERHEAD as u64, MOST_PER_PACKET)
    }

    /// The bit/s the sound, resends and RTCP took over the last two seconds.
    pub fn other(&self, now: Instant) -> u64 {
        let bytes: u64 = [Kind::Resend, Kind::Sound, Kind::Report]
            .iter()
            .map(|&kind| self.meter.recent(now, kind, MEASURED))
            .sum();
        bytes * 8 * 1000 / (MEASURED * SLOT.as_millis() as u64)
    }

    /// The bit/s the encoder may aim for at `fps` frames a second, by what
    /// was measured lately ([`budget`]), before [`Self::trim`].
    pub fn budget(&self, now: Instant, fps: u32) -> u64 {
        budget(self.ceiling, fps, self.per_packet(now), self.other(now))
    }

    /// What went on the wire lately.
    pub fn rates(&self, now: Instant) -> Rates {
        self.meter.rates(now)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    fn ms(milliseconds: u64) -> Duration {
        Duration::from_millis(milliseconds)
    }

    /// The buffer of an encoder held past Discord's documented cap.
    const STRICT: Duration = Duration::from_millis(200);

    /// What went out: when, from the start, and how many bits.
    type Sends = Vec<(Duration, u64)>;

    /// The most bits any window `long` of `sends` (when, from the start, and
    /// bits; in order) holds, of those that start at or after `from`.
    fn most_in(sends: &[(Duration, u64)], long: Duration, from: Duration) -> u64 {
        let (mut most, mut sum, mut end) = (0, 0, 0);
        for &(at, bits) in sends {
            while let Some(&(next, more)) = sends.get(end)
                && next < at + long
            {
                sum += more;
                end += 1;
            }
            if at >= from {
                most = most.max(sum);
            }
            sum -= bits;
        }
        most
    }

    /// The bound the bucket keeps over a window `long`: its depth and its
    /// rate for that long, and the one packet of sound it lets go below
    /// empty.
    fn bound(ceiling: u64, long: Duration, forced: u64) -> u64 {
        depth(ceiling) + rate(ceiling) * long.as_micros() as u64 / 1_000_000 + forced
    }

    /// A busy stream, in steps of 100 µs over `steps`: a keyframe's 400
    /// packets each second and a frame's 40 every 33 ms queued far past
    /// what the ceiling carries, the sound every 20 ms, which never waits,
    /// and a burst of NACKs half a second into each second, asking for a
    /// resend every step for 50 ms; the ceiling `change`s at a step.
    /// Returns what went out, and the resends among it.
    fn stream(ceiling: u64, steps: u64, change: Option<(u64, u64)>) -> (Sends, Sends) {
        let start = Instant::now();
        let mut limiter = Limiter::new(ceiling, STRICT, start);
        let (mut sends, mut resent) = (vec![], vec![]);
        let mut queue: VecDeque<u64> = VecDeque::new();
        for step in 0..steps {
            let at = Duration::from_micros(step * 100);
            let now = start + at;
            if let Some((when, lower)) = change
                && step == when
            {
                limiter.set_ceiling(now, lower, STRICT);
            }
            if step.is_multiple_of(10_000) {
                queue.extend(std::iter::repeat_n(1278, 400));
            }
            if step.is_multiple_of(333) {
                queue.extend(std::iter::repeat_n(1278, 40));
            }
            if step.is_multiple_of(200) {
                limiter.force(now, Kind::Sound, 260);
                sends.push((at, 260 * 8));
            }
            if (5_000..5_500).contains(&(step % 10_000)) && limiter.take_resend(now, 1280) {
                sends.push((at, 1280 * 8));
                resent.push((at, 1280 * 8));
            }
            while let Some(&bytes) = queue.front() {
                if !limiter.take_video(now, bytes) {
                    break;
                }
                queue.pop_front();
                sends.push((at, bytes * 8));
            }
        }
        (sends, resent)
    }

    /// Whatever is queued, resends and sound besides, no window of the
    /// bucket's depth, of 100 ms or of a second sends more than the depth
    /// and the rate for that long allow, which over a second is under the
    /// ceiling, at 16 Mbit/s and at the floor's 500 kbit/s.
    #[test]
    fn never_sends_past_the_ceiling_in_any_window() {
        for ceiling in [16_000_000, 500_000] {
            let (sends, _) = stream(ceiling, 30_000, None);
            let total: u64 = sends.iter().map(|&(_, bits)| bits).sum();
            assert!(total > rate(ceiling) * 2, "it was busy: {total}");
            for long in [DEPTH, ms(100), ms(1000)] {
                let most = most_in(&sends, long, Duration::ZERO);
                assert!(
                    most <= bound(ceiling, long, 260 * 8),
                    "{most} bits in {long:?} at {ceiling}"
                );
            }
            assert!(most_in(&sends, ms(1000), Duration::ZERO) <= ceiling);
        }
        // 16 Mbit/s: any 100 ms sends about 106% of the ceiling's at most,
        // a packet of sound besides.
        let (sends, _) = stream(16_000_000, 30_000, None);
        assert!(most_in(&sends, ms(100), Duration::ZERO) <= 16_000_000 / 10 * 107 / 100);
    }

    /// A burst of NACKs takes at most its share of the bucket, and the
    /// whole still stays under the ceiling.
    #[test]
    fn resends_take_at_most_their_share() {
        let ceiling = 16_000_000;
        let (sends, resent) = stream(ceiling, 20_000, None);
        assert!(!resent.is_empty(), "resends went");
        let share = resend_depth(ceiling) + resend_rate(ceiling) / 20;
        assert!(most_in(&resent, ms(50), Duration::ZERO) <= share);
        assert!(most_in(&sends, ms(1000), Duration::ZERO) <= ceiling);
    }

    /// A ceiling lowered from 16 to 8 Mbit/s holds at once: every window
    /// that starts after the change keeps to the lower one's bound.
    #[test]
    fn a_lower_ceiling_holds_at_once() {
        let (sends, _) = stream(16_000_000, 30_000, Some((15_000, 8_000_000)));
        let changed = ms(1500);
        for long in [ms(100), ms(1000)] {
            let most = most_in(&sends, long, changed);
            assert!(
                most <= bound(8_000_000, long, 260 * 8),
                "{most} in {long:?}"
            );
        }
    }

    #[test]
    fn the_bucket_waits_for_what_it_lacks() {
        let start = Instant::now();
        let mut bucket = Bucket::new(8_000_000, 80_000, start);
        assert!(bucket.take(start, 80_000), "full as it starts");
        assert!(!bucket.take(start, 8_000));
        // 8000 bits at 8 Mbit/s: a millisecond.
        let wait = bucket.wait(start, 8_000);
        assert!(
            (ms(1)..ms(1) + Duration::from_micros(1)).contains(&wait),
            "{wait:?}"
        );
        assert!(bucket.take(start + wait, 8_000));
        // Sound that mustn't wait takes it below empty, and what follows
        // waits the longer.
        bucket.force(start + wait, 8_000);
        assert!(bucket.wait(start + wait, 8_000) > ms(1));
        // A packet larger than the bucket still goes, once it's full.
        let mut small = Bucket::new(1_000_000, 1_000, start);
        assert!(small.take(start, 12_000));
        assert!(small.wait(start, 12_000) > Duration::ZERO);
    }

    /// The meter's rates and peaks, in bit/s, over whole slots.
    #[test]
    fn the_meter_gives_the_last_and_the_most() {
        let start = Instant::now();
        let mut meter = Meter::new(start);
        // 10 kB in each 100 ms for two seconds, 40 kB in one of them.
        for slot in 0..20u64 {
            let bytes = if slot == 7 { 40_000 } else { 10_000 };
            meter.add(start + SLOT * slot as u32, Kind::Video, bytes);
        }
        meter.add(start + SLOT * 3, Kind::Encoded, 1_000_000);
        let rates = meter.rates(start + SLOT * 20);
        assert_eq!(rates.last_100ms, 800_000);
        assert_eq!(rates.last_second, 800_000);
        assert_eq!(rates.peak_100ms, 3_200_000);
        assert_eq!(rates.peak_second, 1_040_000, "the second with the burst");
        // Five seconds later the burst is forgotten.
        assert_eq!(meter.rates(start + SLOT * 80).peak_100ms, 0);
        assert_eq!(meter.recent(start + SLOT * 20, Kind::Video, 10), 100_000);
    }

    /// The encoder aims so that its frames, their packets' overhead, the
    /// sound and RTCP fit in the bucket's rate with room left; a larger
    /// overhead measured, or resends, lower it.
    #[test]
    fn the_encoders_target_leaves_room_for_the_overhead() {
        for (ceiling, fps) in [(16_000_000, 60), (9_000_000, 30), (2_000_000, 15)] {
            let aimed = encoder_ceiling(ceiling, fps);
            let per_packet = ASSUMED_PER_PACKET;
            let wire = aimed * (rtp::CHUNK as u64 + per_packet) / rtp::CHUNK as u64
                + u64::from(fps) * per_packet * 8
                + SOUND;
            assert!(wire <= rate(ceiling) * 96 / 100, "{wire} at {ceiling}");
            assert!(aimed >= ceiling * 80 / 100, "{aimed} of {ceiling}");
        }
        // 16 Mbit/s at 60 fps: about 13.5 Mbit/s of video.
        let aimed = encoder_ceiling(16_000_000, 60);
        assert!((13_300_000..13_700_000).contains(&aimed), "{aimed}");
        assert!(budget(16_000_000, 60, 200, 0) < aimed);
        assert!(budget(16_000_000, 60, ASSUMED_PER_PACKET, 1_000_000) < aimed);
        assert_eq!(
            budget(16_000_000, 60, ASSUMED_PER_PACKET, 0),
            aimed,
            "the sound counts"
        );
    }

    /// The wire a video's rate needs is the least ceiling that leaves the
    /// encoder that much, at any rate and frame rate.
    #[test]
    fn the_wire_for_a_video_leaves_it_that_much() {
        for video in [500_000, 1_000_000, 3_500_000, 9_000_000, 20_000_000] {
            for fps in [15, 30, 60] {
                let wire = wire_for(video, fps);
                assert!(encoder_ceiling(wire, fps) >= video, "{video} at {fps}");
                assert!(
                    encoder_ceiling(wire * 999 / 1000, fps) < video,
                    "{video} at {fps}"
                );
            }
        }
        let wire = wire_for(9_000_000, 60);
        assert!((10_500_000..10_900_000).contains(&wire), "{wire}");
    }

    /// The overhead measured from what was queued, once there's enough.
    #[test]
    fn measures_the_overhead_from_what_was_queued() {
        let start = Instant::now();
        let mut limiter = Limiter::new(16_000_000, STRICT, start);
        assert_eq!(limiter.per_packet(start), ASSUMED_PER_PACKET);
        // A second of 30 frames of 50 kB, each in 46 packets of 80 bytes more.
        for frame in 0..30u32 {
            let at = start + ms(33) * frame;
            limiter.queued_frame(at, 50_000, 50_000 + 46 * 80, 46);
            limiter.force(at, Kind::Sound, 600);
        }
        let now = start + ms(1100);
        assert_eq!(limiter.per_packet(now), 80);
        assert!(
            (70_000..80_000).contains(&limiter.other(now)),
            "{}",
            limiter.other(now)
        );
        assert!(limiter.budget(now, 30) > encoder_ceiling(16_000_000, 30));
    }

    /// A frame that would wait past the most behind others is dropped, one
    /// alone always goes, and an overshoot lowers the encoder's target by as
    /// much at once, then lets it come back once the queue stays short. A
    /// frame encoded while the queue was within the overshoot, and within
    /// the encoder's buffer, always goes; the thresholds follow the buffer.
    #[test]
    fn admits_frames_and_trims_the_encoder_on_overshoot() {
        let start = Instant::now();
        let mut limiter = Limiter::new(8_000_000, STRICT, start);
        let second = limiter.rate();
        let fraction = |share: u64| second * share / 1000;
        let calm = limiter.admit(start, fraction(50), fraction(30));
        assert_eq!(
            calm,
            Admitted {
                send: true,
                trimmed: false
            }
        );
        let fits = limiter.admit(start, fraction(250), fraction(200));
        assert!(
            fits.send,
            "a keyframe of the whole buffer after the gate opened"
        );
        let mut gentle = Limiter::new(8_000_000, Duration::from_millis(500), start);
        let keyframe = gentle.admit(start, fraction(40), fraction(500));
        assert_eq!(
            keyframe,
            Admitted {
                send: true,
                trimmed: false
            }
        );
        assert_eq!(gentle.overshoot(), ms(550));
        let mut limiter = Limiter::new(8_000_000, STRICT, start);
        // A keyframe of 400 ms with nothing before it goes, and trims the
        // target by the share it overshot 250 ms.
        let alone = limiter.admit(start, 0, fraction(400));
        assert_eq!(
            alone,
            Admitted {
                send: true,
                trimmed: true
            }
        );
        assert_eq!(limiter.trim(), 625);
        // Behind 300 ms, a frame of 200 ms is dropped; the target dropped
        // a moment ago, so not again yet.
        let dropped = limiter.admit(start + ms(100), fraction(300), fraction(200));
        assert_eq!(
            dropped,
            Admitted {
                send: false,
                trimmed: false
            }
        );
        // The queue stays short: back a step each half second after a
        // second.
        assert!(!limiter.settle(start + ms(900), 0));
        assert!(limiter.settle(start + ms(1100), 0));
        assert_eq!(limiter.trim(), 675);
        assert!(!limiter.settle(start + ms(1300), 0));
        assert!(
            !limiter.settle(start + ms(1700), fraction(200)),
            "not while it queues"
        );
        assert!(limiter.settle(start + ms(1700), 0));
        assert_eq!(limiter.trim(), 725);
        // Another overshoot of 400 ms lowers it again, no lower than half.
        let over = limiter.admit(start + ms(2000), fraction(100), fraction(300));
        assert_eq!(
            over,
            Admitted {
                send: true,
                trimmed: true
            }
        );
        assert_eq!(limiter.trim(), LEAST_TRIM);
    }
}
