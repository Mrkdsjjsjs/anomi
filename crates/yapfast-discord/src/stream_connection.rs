//! A Go Live stream's media connection, to the server Discord assigned the
//! stream: sending this account's stream, or receiving someone else's. It's
//! a voice connection of its own (`voice_connection.rs`), identifying with
//! the stream's RTC server and video. Yapfast sends H264 and receives VP8 or
//! H264.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc as channel,
    },
    time::Duration,
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::{
    sync::{Notify, mpsc, oneshot},
    time::Instant,
};

use crate::{
    congestion::{self, Controller},
    pacer::{self, Limiter},
    rtp::{self, Assembler, Codec, Orientation, Packetizer},
    stream::{self, StreamServer},
    voice_connection::{Connection, Identity, lock, receive, send_json},
};

/// How often a viewer may ask for a keyframe while frames are lost.
const KEYFRAME_REQUESTS: Duration = Duration::from_millis(500);
/// How often a streamer sends an RTCP sender report.
const SENDER_REPORTS: Duration = Duration::from_secs(1);
/// How often a viewer logs what it received while no picture comes through.
const LOG_EVERY: Duration = Duration::from_secs(5);
/// The most packets waiting to be resent, as many as one NACK may name of
/// those Discord's client resends.
const RESENDS: usize = 128;
/// A resend still waiting this long after it was asked for is given up: the
/// viewer has asked again, or given up itself at 300 ms.
const RESEND_WITHIN: Duration = Duration::from_millis(250);
/// The most video SSRCs a watched stream puts frames together for.
const MAX_SSRCS: usize = 4;
/// The sender reports the last round trips are measured against, by the
/// time in them a receiver report echoes.
const ECHOED: usize = 4;
/// How often a sender takes stock: the overhead measured moves the
/// encoder's target, an encoder that overshot gets its target back, and
/// the rates on the wire go to the call's details.
const STATS_EVERY: Duration = Duration::from_millis(500);
/// The least bit/s the encoder is asked for, however low a ceiling leaves it.
const LEAST_TARGET: u64 = 100_000;

/// The most bit/s Discord's documentation shows a stream sending: op 12's
/// `max_bitrate` in the voice docs' example, 9 Mbit/s at 60 fps, which is
/// also what Discord's client sends above 720p.
pub const DOCUMENTED_MAX: u32 = 9_000_000;
/// The most a bitrate limit may be set to with the unlock (Settings,
/// Advanced, "Unlocked bitrate"): past what Discord documents, where its
/// servers may cut the stream, lose its packets or drop it.
pub const UNLOCKED_MAX: u32 = 50_000_000;

/// The most bit/s a stream of `height` lines may take, as Discord's client
/// decides it: 3.5 Mbit/s up to 720p, 9 Mbit/s above, 1440p and the
/// source's size too, at any frame rate. The voice docs give 9 Mbit/s as
/// op 12's `max_bitrate` at 60 fps and name no other cap.
pub fn max_bitrate(height: u32) -> u32 {
    if height > 720 {
        DOCUMENTED_MAX
    } else {
        3_500_000
    }
}

/// The most bit/s a stream of `height` lines is sent at, with the bitrate
/// limit the person chose, `max_kbps`, none for Auto: Discord's cap for the
/// height, or the limit when that's lower, never below
/// [`congestion::FLOOR`]. Without the unlock a limit only lowers it, and
/// one kept above it counts as Discord's cap. With it, `unlocked`, the
/// limit is the cap at every size, up to [`UNLOCKED_MAX`]; Auto stays
/// Discord's. Op 12 announces it. A limit the person set is the ceiling on
/// the wire as it is ([`wire_ceiling`]); the encoder and the congestion
/// controller stay under what that leaves the video ([`encoder_cap`]).
pub fn cap(height: u32, max_kbps: Option<u32>, unlocked: bool) -> u32 {
    let discord = max_bitrate(height);
    let most = if unlocked { UNLOCKED_MAX } else { discord };
    max_kbps.map_or(discord, |kbps| {
        kbps.saturating_mul(1000)
            .clamp(congestion::FLOOR as u32, most)
    })
}

/// Whether the bitrate limit the person chose is the [`cap`], rather than
/// Discord's cap for the height: one at or under Discord's, or any with the
/// unlock.
fn limited(height: u32, max_kbps: Option<u32>, unlocked: bool) -> bool {
    max_kbps.is_some_and(|kbps| unlocked || kbps.saturating_mul(1000) <= max_bitrate(height))
}

/// The ceiling on the wire, in bit/s, for a stream of `height` lines at
/// `fps` frames a second: everything the stream connection sends in any
/// second stays under it ([`pacer`]). A limit the person chose is the
/// ceiling as it is ([`cap`]). Discord's own cap is the video's, as its
/// client sends it, and the wire takes what that video needs besides
/// ([`pacer::wire_for`]): about 10.7 Mbit/s for 9.
pub fn wire_ceiling(height: u32, max_kbps: Option<u32>, unlocked: bool, fps: u32) -> u64 {
    let cap = u64::from(cap(height, max_kbps, unlocked));
    if limited(height, max_kbps, unlocked) {
        cap
    } else {
        pacer::wire_for(cap, fps)
    }
}

/// The most bit/s the encoder aims for under [`cap`], at `fps` frames a
/// second, before the stream measured its overhead: what the ceiling on the
/// wire leaves the video once the packets' headers, the transport's
/// sealing, the stream's sound and the limiter's margins are taken
/// ([`pacer::encoder_ceiling`]), and never past the cap.
pub fn encoder_cap(height: u32, max_kbps: Option<u32>, unlocked: bool, fps: u32) -> u64 {
    let wire = wire_ceiling(height, max_kbps, unlocked, fps);
    pacer::encoder_ceiling(wire, fps).min(cap(height, max_kbps, unlocked).into())
}

/// The rate control buffer of an encoder held under a cap past Discord's
/// documented 9 Mbit/s, where its servers were seen to break a stream for
/// going over: a keyframe or a scene cut waits in the limiter's queue a
/// fifth of a second at most.
pub const STRICT_BUFFER: Duration = Duration::from_millis(200);
/// Under Discord's cap and below: half a second, which keeps a screen's
/// text sharp on each keyframe at a few Mbit/s, for up to half a second's
/// wait behind one.
pub const GENTLE_BUFFER: Duration = Duration::from_millis(500);

/// The rate control buffer the stream's encoder keeps under [`cap`]
/// ([`STRICT_BUFFER`], [`GENTLE_BUFFER`]); the limiter's thresholds follow
/// it ([`pacer::overshoot`]).
pub fn encoder_buffer(height: u32, max_kbps: Option<u32>, unlocked: bool) -> Duration {
    if cap(height, max_kbps, unlocked) > DOCUMENTED_MAX {
        STRICT_BUFFER
    } else {
        GENTLE_BUFFER
    }
}

/// Bit/s Discord's servers were seen to take from one account's stream, on
/// 2026-11-20, which Discord doesn't document: a ceiling of 16 Mbit/s held,
/// and the moments the stream went to 17 to 19 broke it.
pub const OBSERVED_MAX: u32 = 16_000_000;

/// Op 5's speaking flags for a stream's sound: shared sound, not a
/// microphone, as Discord's client marks it.
const SOUNDSHARE: u8 = 2;

/// The codec every Yapfast sender can stream in, as every other Go Live
/// sender known to work does. A GPU that encodes AV1 or H265 offers those
/// first.
pub const SEND_CODEC: Codec = Codec::H264;

/// One encoded frame to send, in `codec`, with its time in the 90 kHz clock.
pub struct Frame {
    pub data: Vec<u8>,
    pub codec: Codec,
    pub timestamp: u32,
    pub keyframe: bool,
}

/// One Opus frame of the stream's sound: 20 ms of stereo at 48 kHz, with
/// `timestamp` in samples.
pub struct Sound {
    pub opus: Vec<u8>,
    pub timestamp: u32,
}

/// Queues the stream's sound, from a thread of the sender's own.
#[derive(Clone)]
pub struct SoundSender(mpsc::Sender<Sound>);

impl SoundSender {
    /// Queues a frame of sound, dropping it if the network can't keep up.
    pub fn send(&self, sound: Sound) {
        let _ = self.0.try_send(sound);
    }
}

/// One whole frame received, decrypted and ready to decode.
pub struct ReceivedFrame {
    pub codec: Codec,
    pub data: Vec<u8>,
    pub keyframe: bool,
    /// How to turn the decoded picture upright, as the sender last said.
    pub orientation: Orientation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Connected,
    Ended(String),
}

struct Shared {
    link: Mutex<Link>,
    /// Sending: the next frame should be a keyframe. Receiving: ask the
    /// streamer for one.
    keyframe: AtomicBool,
    size: Mutex<(u32, u32)>,
    /// Sending: the frames a second Discord is told of, which may change
    /// while the stream goes.
    fps: AtomicU32,
    /// Sending: the bit/s to encode at, as [`Controller`] decides.
    target: AtomicU64,
    /// Sending: the bitrate limit the person chose, in kbit/s, 0 for none,
    /// with [`UNLOCKED_BIT`] set while it may pass Discord's documented cap
    /// ([`cap`]): one word, so the sending thread never reads a limit with
    /// another's unlock.
    ceiling: AtomicU32,
    /// Sending: the codecs offered, most wanted first, and whether that
    /// changed since Discord was told. Receiving: the codecs decoded.
    offers: Mutex<Vec<Codec>>,
    offers_changed: AtomicBool,
    /// Sending: the codec Discord chose, in op 4 and then op 14, once it
    /// said.
    chosen: Mutex<Option<Codec>>,
    /// Sending: the video's bytes sent so far, resent ones too, as UDP
    /// payloads; the keyframes viewers asked for; the packets resent; and
    /// the frames dropped whole because the limiter couldn't send them in
    /// time.
    video_bytes: AtomicU64,
    keyframe_requests: AtomicU64,
    resent: AtomicU64,
    dropped: AtomicU64,
    /// Sending: how long what's queued takes to go out under the ceiling,
    /// and how long it may before the encoder has overshot
    /// ([`pacer::overshoot`]), in microseconds.
    backlog: AtomicU64,
    overshoot: AtomicU64,
    /// Sending: what went on the wire lately, everything counted, in bit/s
    /// ([`pacer::Rates`]), and the ceiling it's held under.
    wire: AtomicU64,
    peak_100ms: AtomicU64,
    peak_second: AtomicU64,
    ceiling_bps: AtomicU64,
    /// Receiving: whether the stream's video is wanted, as
    /// [`StreamReceiver::want_video`] last said; Discord hears it with op 15.
    video_wanted: AtomicBool,
    /// Receiving: told as the video's want changes, so a connection that
    /// waits on nothing else hears it at once.
    wants_changed: Notify,
    /// Receiving: the packets of the stream's sound so far.
    sounds: AtomicU64,
}

impl Shared {
    fn new(size: (u32, u32)) -> Arc<Self> {
        // The frame rate isn't known yet: the most frames' overhead.
        let most = encoder_cap(size.1, None, false, 60);
        let overshoot = pacer::overshoot(encoder_buffer(size.1, None, false));
        Arc::new(Self {
            link: Mutex::new(Link::Connecting),
            keyframe: AtomicBool::new(true),
            size: Mutex::new(size),
            fps: AtomicU32::new(0),
            target: AtomicU64::new(congestion::start(most)),
            ceiling: AtomicU32::new(0),
            offers: Mutex::new(vec![SEND_CODEC]),
            offers_changed: AtomicBool::new(false),
            chosen: Mutex::new(None),
            video_bytes: AtomicU64::new(0),
            keyframe_requests: AtomicU64::new(0),
            resent: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            backlog: AtomicU64::new(0),
            overshoot: AtomicU64::new(overshoot.as_micros() as u64),
            wire: AtomicU64::new(0),
            peak_100ms: AtomicU64::new(0),
            peak_second: AtomicU64::new(0),
            ceiling_bps: AtomicU64::new(0),
            video_wanted: AtomicBool::new(true),
            wants_changed: Notify::new(),
            sounds: AtomicU64::new(0),
        })
    }

    /// The bitrate limit the person chose, in kbit/s, and whether it may
    /// pass Discord's documented cap.
    fn ceiling(&self) -> (Option<u32>, bool) {
        let packed = self.ceiling.load(Ordering::Relaxed);
        let kbps = packed & !UNLOCKED_BIT;
        ((kbps != 0).then_some(kbps), packed & UNLOCKED_BIT != 0)
    }

    /// The most bit/s a frame `height` lines tall is sent at, with the
    /// limit chosen and its unlock ([`cap`]).
    fn cap(&self, height: u32) -> u32 {
        let (max_kbps, unlocked) = self.ceiling();
        cap(height, max_kbps, unlocked)
    }

    /// The ceiling on the wire for frames `height` lines tall, `fps` a
    /// second ([`wire_ceiling`]), and what it leaves the video
    /// ([`encoder_cap`]).
    fn wire_ceiling(&self, height: u32, fps: u32) -> u64 {
        let (max_kbps, unlocked) = self.ceiling();
        wire_ceiling(height, max_kbps, unlocked, fps)
    }

    fn encoder_cap(&self, height: u32, fps: u32) -> u64 {
        let (max_kbps, unlocked) = self.ceiling();
        encoder_cap(height, max_kbps, unlocked, fps)
    }

    /// The encoder's rate control buffer for frames `height` lines tall
    /// ([`encoder_buffer`]).
    fn buffer(&self, height: u32) -> Duration {
        let (max_kbps, unlocked) = self.ceiling();
        encoder_buffer(height, max_kbps, unlocked)
    }
}

/// [`Shared::ceiling`]'s mark of a limit that may pass Discord's
/// documented cap; the kbit/s below it.
const UNLOCKED_BIT: u32 = 1 << 31;

/// Runs a connection on a thread of its own until it ends, recording why.
fn spawn(
    name: &str,
    shared: Arc<Shared>,
    run: impl FnOnce(Arc<Shared>) -> std::pin::Pin<Box<dyn Future<Output = Result<()>>>>
    + Send
    + 'static,
) -> Result<()> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(anyhow::Error::from)
                .and_then(|runtime| runtime.block_on(run(shared.clone())));
            let reason = match result {
                Ok(()) => "stopped".to_owned(),
                Err(error) => {
                    tracing::warn!(error = format!("{error:#}"), "the stream connection ended");
                    format!("{error:#}")
                }
            };
            *lock(&shared.link) = Link::Ended(reason);
        })
        .context("could not start the stream connection")?;
    Ok(())
}

/// The user and channel ids a stream key names, as numbers.
fn ids(server: &StreamServer) -> Result<(u64, u64, u64)> {
    let (_, channel, streamer) = stream::parse(&server.stream_key).context("not a stream key")?;
    Ok((
        server.user_id.parse().context("not a user id")?,
        channel.parse().context("not a channel id")?,
        streamer.parse().context("not a user id")?,
    ))
}

/// Sends frames on a thread of its own; dropping it ends the connection.
pub struct StreamSender {
    frames: mpsc::Sender<Frame>,
    sounds: mpsc::Sender<Sound>,
    shared: Arc<Shared>,
}

impl StreamSender {
    /// Connects to `server` and announces `width` by `height` video,
    /// offering to send in `offers`, most wanted first. Discord picks the
    /// first every viewer can decode; see [`Self::codec`].
    pub fn start(
        server: StreamServer,
        width: u32,
        height: u32,
        fps: u32,
        offers: Vec<Codec>,
    ) -> Result<Self> {
        let (user, channel, _) = ids(&server)?;
        let shared = Shared::new((width, height));
        shared.fps.store(fps, Ordering::Relaxed);
        *lock(&shared.offers) = offers;
        // A few frames of slack; beyond that the network can't keep up.
        let (frames, receiver) = mpsc::channel(4);
        // 160 ms of sound.
        let (sounds, heard) = mpsc::channel(8);
        spawn("yapfast-stream-send", shared.clone(), move |shared| {
            Box::pin(
                async move { send(&server, user, channel, fps, &shared, receiver, heard).await },
            )
        })?;
        Ok(Self {
            frames,
            sounds,
            shared,
        })
    }

    /// Where the stream's sound goes.
    pub fn sound(&self) -> SoundSender {
        SoundSender(self.sounds.clone())
    }

    /// Queues a frame. A full queue drops it and asks for a keyframe, so
    /// viewers recover once the network catches up.
    pub fn send(&self, frame: Frame) {
        if self.frames.try_send(frame).is_err() {
            self.shared.keyframe.store(true, Ordering::Relaxed);
        }
    }

    /// Announces a new frame size with the next frame.
    pub fn resize(&self, width: u32, height: u32) {
        *lock(&self.shared.size) = (width, height);
    }

    /// Announces a new frame rate with the next frame, and paces frames by it.
    pub fn set_fps(&self, fps: u32) {
        self.shared.fps.store(fps, Ordering::Relaxed);
    }

    /// Sends at most `max_kbps` from now on, none for Auto, within Discord's
    /// cap for the stream's height unless `unlocked` lets it past, up to
    /// [`UNLOCKED_MAX`] ([`cap`]): the encoder's target drops under what it
    /// leaves the video at once ([`encoder_cap`]), and the sending thread,
    /// as it next wakes, holds the wire under it, announces it with op 12
    /// and caps the congestion controller. A limit raised again, or
    /// unlocked, lets the target rise as the uplink allows, never at once.
    pub fn set_ceiling(&self, max_kbps: Option<u32>, unlocked: bool) {
        let kbps = max_kbps.unwrap_or(0).min(!UNLOCKED_BIT);
        let mark = if unlocked { UNLOCKED_BIT } else { 0 };
        self.shared.ceiling.store(kbps | mark, Ordering::Relaxed);
        let height = lock(&self.shared.size).1;
        let fps = self.shared.fps.load(Ordering::Relaxed);
        let most = self.shared.encoder_cap(height, fps);
        self.shared.target.fetch_min(most, Ordering::Relaxed);
    }

    /// Whether what's queued would take longer than [`pacer::overshoot`] to
    /// go out under the ceiling: the encoder sent more than its target, and
    /// the stream worker skips encoding frames until the queue is shorter,
    /// rather than make frames the limiter would drop.
    pub fn behind(&self) -> bool {
        let overshoot = self.shared.overshoot.load(Ordering::Relaxed);
        self.shared.backlog.load(Ordering::Relaxed) > overshoot
    }

    /// Whether the next frame should be a keyframe: at the start, after
    /// dropped frames, or when a viewer asked for one.
    pub fn take_keyframe_request(&self) -> bool {
        self.shared.keyframe.swap(false, Ordering::Relaxed)
    }

    /// The bit/s to encode at: what keeps the uplink's queue short, within
    /// Discord's cap and its server's estimate. See [`congestion`].
    pub fn target(&self) -> u64 {
        self.shared.target.load(Ordering::Relaxed)
    }

    pub fn link(&self) -> Link {
        lock(&self.shared.link).clone()
    }

    /// The codec Discord wants the frames in, once it said. It changes when
    /// a viewer joins who can't decode it, or leaves; frames in another
    /// codec are dropped.
    pub fn codec(&self) -> Option<Codec> {
        let chosen = (*lock(&self.shared.chosen))?;
        let offers = lock(&self.shared.offers);
        // Withdrawn, while Discord hasn't chosen again yet.
        Some(chosen)
            .filter(|chosen| offers.contains(chosen))
            .or(offers.first().copied())
    }

    /// What the stream sent so far, and what viewers asked of it.
    pub fn sent(&self) -> SentSoFar {
        let load = |count: &AtomicU64| count.load(Ordering::Relaxed);
        SentSoFar {
            video_bytes: load(&self.shared.video_bytes),
            keyframe_requests: load(&self.shared.keyframe_requests),
            resent: load(&self.shared.resent),
            dropped: load(&self.shared.dropped),
            wire: load(&self.shared.wire),
            peak_100ms: load(&self.shared.peak_100ms),
            peak_second: load(&self.shared.peak_second),
            ceiling: load(&self.shared.ceiling_bps),
        }
    }

    /// Stops offering `codec`, which the encoder can't produce any more:
    /// Discord then picks another, which [`Self::codec`] gives.
    pub fn withdraw(&self, codec: Codec) {
        let mut offers = lock(&self.shared.offers);
        if offers.len() > 1 && offers.contains(&codec) {
            offers.retain(|&offered| offered != codec);
            self.shared.offers_changed.store(true, Ordering::Relaxed);
        }
    }
}

/// A stream's video sent since it started, as [`StreamSender::sent`] says:
/// bytes as UDP payloads, resent ones too; keyframes viewers asked for;
/// packets resent when a viewer reported them lost; and frames dropped
/// whole because the limiter couldn't send them in time. Then what the
/// stream connection put on the wire lately, everything counted
/// ([`pacer`]), in bit/s: over the last second, and the most any 100 ms
/// and any second of the last five carried; and the ceiling it's held
/// under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SentSoFar {
    pub video_bytes: u64,
    pub keyframe_requests: u64,
    pub resent: u64,
    pub dropped: u64,
    pub wire: u64,
    pub peak_100ms: u64,
    pub peak_second: u64,
    pub ceiling: u64,
}

/// Receives someone's stream on a thread of its own; dropping it ends the
/// connection.
pub struct StreamReceiver {
    frames: channel::Receiver<ReceivedFrame>,
    sounds: Option<channel::Receiver<Vec<u8>>>,
    shared: Arc<Shared>,
    _stop: oneshot::Sender<()>,
}

impl StreamReceiver {
    /// Watches the stream, telling Discord the viewer decodes `decodes`,
    /// most wanted first.
    pub fn start(server: StreamServer, decodes: Vec<Codec>) -> Result<Self> {
        let (user, channel, streamer) = ids(&server)?;
        let shared = Shared::new((0, 0));
        *lock(&shared.offers) = decodes;
        // Frames waiting to be decoded; more means decoding can't keep up.
        let (frames, receiver) = channel::sync_channel(4);
        // Opus frames waiting to be played: 320 ms.
        let (sounds, heard) = channel::sync_channel(16);
        let (stop, stopped) = oneshot::channel();
        spawn("yapfast-stream-receive", shared.clone(), move |shared| {
            Box::pin(async move {
                receive_stream(
                    &server,
                    user,
                    channel,
                    streamer,
                    &shared,
                    (frames, sounds),
                    stopped,
                )
                .await
            })
        })?;
        Ok(Self {
            frames: receiver,
            sounds: Some(heard),
            shared,
            _stop: stop,
        })
    }

    /// The next whole frame, waiting at most `wait`.
    pub fn next(&self, wait: Duration) -> Option<ReceivedFrame> {
        self.frames.recv_timeout(wait).ok()
    }

    /// The stream's sound, decrypted Opus frames, for a player of its own.
    /// It ends with the connection.
    pub fn take_sounds(&mut self) -> Option<channel::Receiver<Vec<u8>>> {
        self.sounds.take()
    }

    /// Asks the streamer for a keyframe, as after a frame that wouldn't decode.
    pub fn request_keyframe(&self) {
        self.shared.keyframe.store(true, Ordering::Relaxed);
    }

    /// Asks Discord for the stream's video, or for none of it while nobody
    /// sees its picture: op 15 then wants the video's SSRC at 0, and
    /// everything else, the stream's sound on an SSRC of its own, in full
    /// (`sink_wants`). Wanted again, the video is put together afresh from
    /// a keyframe the connection asks for at once.
    pub fn want_video(&self, wanted: bool) {
        self.shared.video_wanted.store(wanted, Ordering::Relaxed);
        self.shared.wants_changed.notify_one();
    }

    /// The packets of the stream's sound received so far.
    pub fn sounds_received(&self) -> u64 {
        self.shared.sounds.load(Ordering::Relaxed)
    }

    pub fn link(&self) -> Link {
        lock(&self.shared.link).clone()
    }
}

/// Op 15's wants of a watched stream: its video at full quality, or at 0
/// while it isn't `wanted`. `any` covers the SSRCs left out, the stream's
/// sound among them, which always comes in full.
fn sink_wants(video: u32, wanted: bool) -> Value {
    let mut wants = json!({"any": 100});
    wants[video.to_string()] = json!(if wanted { 100 } else { 0 });
    wants
}

/// Op 12's video state for a camera, capped as Discord's client caps its
/// height.
pub(crate) fn video_attributes(
    ssrc: u32,
    video: u32,
    rtx: u32,
    size: (u32, u32),
    fps: u32,
) -> Value {
    attributes(ssrc, video, rtx, size, fps, max_bitrate(size.1))
}

/// Op 12's video state, announcing `bitrate` as its `max_bitrate`.
fn attributes(
    ssrc: u32,
    video: u32,
    rtx: u32,
    (width, height): (u32, u32),
    fps: u32,
    bitrate: u32,
) -> Value {
    json!({
        "audio_ssrc": ssrc,
        "video_ssrc": video,
        "rtx_ssrc": rtx,
        "streams": [{
            "type": "video",
            "rid": rtp::STREAM_RID,
            "ssrc": video,
            "active": true,
            "quality": 100,
            "rtx_ssrc": rtx,
            "max_bitrate": bitrate,
            "max_framerate": fps,
            "max_resolution": {"type": "fixed", "width": width, "height": height},
        }],
    })
}

/// Which way the video goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Send,
    Receive,
}

/// Connects to a stream's server: identifying with its RTC server and a
/// screen, and offering the codecs `role` needs.
/// Select protocol's codecs for a sender: Opus, then each of `offers` by
/// priority, and `withdrawn` ones it can't encode any more. A sender
/// decodes nothing of its own stream.
fn send_codecs(offers: &[Codec], withdrawn: &[Codec]) -> Value {
    let video = offers
        .iter()
        .map(|&codec| (codec, true))
        .chain(withdrawn.iter().map(|&codec| (codec, false)));
    codecs(video.map(|(codec, encode)| (codec, encode, matches!(codec, Codec::Vp8 | Codec::H264))))
}

/// Select protocol's codecs for a viewer: Opus, then each of `decodes` by
/// priority, to decode only.
fn receive_codecs(decodes: &[Codec]) -> Value {
    codecs(decodes.iter().map(|&codec| (codec, false, true)))
}

/// Opus, then each video codec with whether it's encoded and decoded, most
/// wanted first.
fn codecs(video: impl Iterator<Item = (Codec, bool, bool)>) -> Value {
    let mut codecs =
        vec![json!({"name": "opus", "type": "audio", "priority": 1000, "payload_type": rtp::OPUS})];
    for (index, (codec, encode, decode)) in video.enumerate() {
        let (payload_type, rtx_payload_type) = codec.payload_types();
        codecs.push(json!({
            "name": codec.name(), "type": "video", "priority": 1000 * (index + 1),
            "payload_type": payload_type, "rtx_payload_type": rtx_payload_type,
            "encode": encode, "decode": decode,
        }));
    }
    Value::Array(codecs)
}

async fn open(
    server: &StreamServer,
    user: u64,
    channel: u64,
    role: Role,
    shared: &Shared,
) -> Result<Connection> {
    // Discord made a stream's DAVE group id just before its RTC server id,
    // in the one stream seen so far; Dave moves to the id the voice server
    // uses if this guess is wrong.
    let group = server
        .rtc_server_id
        .parse::<u64>()
        .map_or(channel, |id| id.saturating_sub(1));
    let identity = Identity {
        endpoint: server.endpoint.clone(),
        server_id: server.rtc_server_id.clone(),
        user_id: server.user_id.clone(),
        session_id: server.session_id.clone(),
        token: server.token.clone(),
        streams: json!([{"type": "screen", "rid": rtp::STREAM_RID, "quality": 100}]),
        user,
        group,
    };
    // Yapfast encodes in what it offers, and decodes what it was given.
    let codecs = match role {
        Role::Send => send_codecs(&lock(&shared.offers), &[]),
        Role::Receive => receive_codecs(&lock(&shared.offers)),
    };
    let streamer: Vec<u64> = stream::parse(&server.stream_key)
        .and_then(|(_, _, streamer)| streamer.parse().ok())
        .into_iter()
        .collect();
    let connection = Connection::open(&identity, codecs, &streamer).await?;
    tracing::info!(rtc_server = %server.rtc_server_id, channel, "connected to a Discord stream");
    Ok(connection)
}

async fn send(
    server: &StreamServer,
    user: u64,
    channel: u64,
    mut fps: u32,
    shared: &Shared,
    mut frames: mpsc::Receiver<Frame>,
    mut sounds: mpsc::Receiver<Sound>,
) -> Result<()> {
    let mut connection = open(server, user, channel, Role::Send, shared).await?;
    let (ssrc, video, rtx) = (connection.ssrc, connection.video, connection.rtx);
    let mut size = *lock(&shared.size);
    // The cap op 12 announces: Discord's for the size, or the person's
    // limit below it, or past it with the unlock. The limiter holds every
    // packet under the ceiling on the wire it makes ([`wire_ceiling`]);
    // the encoder and the congestion controller aim under what that leaves
    // the video.
    let now = std::time::Instant::now();
    let mut limiter = Limiter::new(shared.wire_ceiling(size.1, fps), shared.buffer(size.1), now);
    if connection
        .udp
        .peer_addr()
        .is_ok_and(|address| address.is_ipv6())
    {
        limiter.set_datagram(pacer::DATAGRAM_V6);
    }
    let mut pacing = Controller::new(shared.encoder_cap(size.1, fps));
    let mut history = rtp::History::default();
    let mut capped = recap(size.1, fps, shared, &mut limiter, &mut pacing, &mut history);
    // When the last video sender reports went out, by the time in them.
    let mut echoed = [(0u32, std::time::Instant::now()); ECHOED];
    let mut reports_sent = 0usize;
    send_json(
        &mut connection.ws,
        12,
        attributes(ssrc, video, rtx, size, fps, capped),
    )
    .await?;
    send_json(
        &mut connection.ws,
        5,
        json!({"speaking": SOUNDSHARE, "delay": 0, "ssrc": ssrc}),
    )
    .await?;
    *lock(&shared.link) = Link::Connected;

    // Discord's choice, or the first offered while it hasn't said.
    let offered = lock(&shared.offers).clone();
    let chosen = connection.codec.unwrap_or(offered[0]);
    if !offered.contains(&chosen) {
        bail!("Discord asked for {chosen:?} video, and Yapfast offered {offered:?}");
    }
    *lock(&shared.chosen) = Some(chosen);
    tracing::info!(codec = ?chosen, ?offered, "Discord chose the stream's codec");
    let mut withdrawn: Vec<Codec> = vec![];
    let mut packetizer = Packetizer::new(video, chosen);
    let interval = connection.interval;
    let mut heartbeat = tokio::time::interval_at(Instant::now() + interval, interval);
    let mut datagram = vec![0u8; 1500];
    // For RTCP sender reports: packets and payload bytes sent so far.
    let (mut packets, mut octets) = (0u32, 0u32);
    let mut reported = Instant::now() - SENDER_REPORTS;
    // For the log: whether frames went out encrypted, and what came back.
    let mut encrypted = None;
    let mut sent = Sent::default();
    let mut log = tokio::time::interval_at(Instant::now() + LOG_EVERY, LOG_EVERY);
    let mut stats = tokio::time::interval(STATS_EVERY);
    // The sound's RTP numbering and sender report counts, on the audio SSRC.
    let mut sound_sequence: u16 = rand::random();
    let (mut sound_packets, mut sound_octets) = (0u32, 0u32);
    let mut sound_reported = Instant::now() - SENDER_REPORTS;
    // The packets of the frames queued, which go out from their own branch
    // as the limiter lets them, so receiver reports are timed as they come;
    // their bits on the wire; and the RTP timestamp of the last frame.
    let mut pending: VecDeque<Vec<u8>> = VecDeque::new();
    let mut queued = 0u64;
    let mut pending_timestamp = 0u32;
    // The packets viewers asked for again, with when they asked, sent
    // before the frames' as the limiter lets them.
    let mut resends: VecDeque<(u16, std::time::Instant)> = VecDeque::with_capacity(RESENDS);
    // A frame was dropped: those after it predict from it, and go too,
    // until a keyframe.
    let mut until_keyframe = false;
    loop {
        // A new bitrate limit takes effect as the loop next wakes, whatever
        // woke it: the limiter at once, op 12 with it.
        if (shared.cap(size.1), shared.wire_ceiling(size.1, fps)) != (capped, limiter.ceiling()) {
            capped = recap(size.1, fps, shared, &mut limiter, &mut pacing, &mut history);
            send_json(
                &mut connection.ws,
                12,
                attributes(ssrc, video, rtx, size, fps, capped),
            )
            .await?;
        }
        let wake = next_send(&mut limiter, &pending, queued, &resends, &history);
        tokio::select! {
            frame = frames.recv() => {
                let Some(frame) = frame else {
                    let _ = connection.ws.close(None).await;
                    return Ok(());
                };
                // A new size or frame rate, announced before the first
                // frame that has it.
                let (wanted_size, wanted_fps) = (*lock(&shared.size), shared.fps.load(Ordering::Relaxed));
                if (wanted_size, wanted_fps) != (size, fps) {
                    (size, fps) = (wanted_size, wanted_fps);
                    capped = recap(size.1, fps, shared, &mut limiter, &mut pacing, &mut history);
                    send_json(&mut connection.ws, 12, attributes(ssrc, video, rtx, size, fps, capped)).await?;
                }
                let encrypting = connection.dave.encrypting();
                if encrypted != Some(encrypting) {
                    encrypted = Some(encrypting);
                    tracing::info!(
                        end_to_end = encrypting,
                        dave = connection.dave.status(),
                        "sending stream frames"
                    );
                }
                if shared.offers_changed.swap(false, Ordering::Relaxed) {
                    let offers = lock(&shared.offers).clone();
                    withdrawn = offered.iter().copied().filter(|codec| !offers.contains(codec)).collect();
                    tracing::info!(?offers, ?withdrawn, "offering fewer codecs");
                    send_json(&mut connection.ws, 14, json!({"codecs": send_codecs(&offers, &withdrawn)})).await?;
                }
                // Frames encoded before the encoder followed Discord's choice.
                if Some(frame.codec) != *lock(&shared.chosen) {
                    continue;
                }
                // Viewers would decode it against the frame dropped before.
                if until_keyframe && !frame.keyframe {
                    sent.dropped += 1;
                    shared.dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                packetizer.set_codec(frame.codec);
                let Some(payload) = connection.dave.encrypt(frame.codec, &frame.data) else {
                    continue;
                };
                // Whole or not at all: a frame that would wait too long
                // behind those queued is dropped before it takes sequence
                // numbers, with those after it until a keyframe.
                let now = std::time::Instant::now();
                let admitted = limiter.admit(now, queued, pacer::frame_on_wire(payload.len()) * 8);
                if admitted.trimmed {
                    let queued_ms = limiter.queued_for(queued).as_millis() as u64;
                    tracing::debug!(trim = limiter.trim(), queued_ms, frame = payload.len(), "the encoder overshot what the ceiling lets out; lowering its target");
                    publish(shared, &pacing, &limiter, fps);
                }
                until_keyframe = !admitted.send;
                if !admitted.send {
                    sent.dropped += 1;
                    shared.dropped.fetch_add(1, Ordering::Relaxed);
                    shared.keyframe.store(true, Ordering::Relaxed);
                    continue;
                }
                sent.frames += 1;
                sent.keyframes += u32::from(frame.keyframe);
                let packets_of_frame = packetizer.packets(&payload, frame.timestamp, frame.keyframe);
                let wire: u64 = packets_of_frame
                    .iter()
                    .map(|packet| limiter.on_wire(packet.len() + rtp::SEALING))
                    .sum();
                limiter.queued_frame(now, frame.data.len(), wire, packets_of_frame.len());
                queued += wire * 8;
                pending.extend(packets_of_frame);
                pending_timestamp = frame.timestamp;
                shared.backlog.store(limiter.queued_for(queued).as_micros() as u64, Ordering::Relaxed);
            }
            _ = tokio::time::sleep(wake.unwrap_or_default()), if wake.is_some() => {
                let now = std::time::Instant::now();
                // Resends first, within their share of the ceiling; those
                // asked for too long ago, or gone from the history, are
                // given up.
                while let Some(&(lost, asked)) = resends.front() {
                    let Some(original) = history
                        .get(lost)
                        .filter(|_| now.saturating_duration_since(asked) <= RESEND_WITHIN)
                    else {
                        resends.pop_front();
                        sent.too_late += 1;
                        continue;
                    };
                    // RTX puts the original sequence number before the payload.
                    if !limiter.take_resend(now, limiter.on_wire(original.len() + 2 + rtp::SEALING)) {
                        break;
                    }
                    resends.pop_front();
                    let Some(mut again) = packetizer.retransmit(original, rtx) else {
                        continue;
                    };
                    let clear = rtp::rtp_clear_len(&again).unwrap_or(again.len());
                    connection.transport.seal(&mut again, clear)?;
                    connection.udp.send(&again).await?;
                    shared.video_bytes.fetch_add(again.len() as u64, Ordering::Relaxed);
                    pacing.sent(now, again.len());
                    packetizer.recycle(again);
                    shared.resent.fetch_add(1, Ordering::Relaxed);
                    sent.resent += 1;
                }
                // Then the frames' packets, as many as the limiter holds.
                while let Some(front) = pending.front() {
                    let bytes = limiter.on_wire(front.len() + rtp::SEALING);
                    if !limiter.take_video(now, bytes) {
                        break;
                    }
                    let Some(mut packet) = pending.pop_front() else {
                        break;
                    };
                    queued = queued.saturating_sub(bytes * 8);
                    history.remember(&packet);
                    let clear = rtp::rtp_clear_len(&packet).unwrap_or(packet.len());
                    packets = packets.wrapping_add(1);
                    octets = octets.wrapping_add((packet.len() - clear) as u32);
                    connection.transport.seal(&mut packet, clear)?;
                    connection.udp.send(&packet).await?;
                    shared.video_bytes.fetch_add(packet.len() as u64, Ordering::Relaxed);
                    pacing.sent(now, packet.len());
                    // The next frame's packets are built in it.
                    packetizer.recycle(packet);
                }
                if pending.is_empty() {
                    queued = 0;
                }
                shared.backlog.store(limiter.queued_for(queued).as_micros() as u64, Ordering::Relaxed);
                if pending.is_empty() && reported.elapsed() >= SENDER_REPORTS {
                    reported = Instant::now();
                    let mut report = rtp::sender_report(
                        video,
                        std::time::SystemTime::now(),
                        pending_timestamp,
                        packets,
                        octets,
                    );
                    if let Some(&(_, middle)) = rtp::sender_reports(&report).first() {
                        echoed[reports_sent % ECHOED] = (middle, std::time::Instant::now());
                        reports_sent += 1;
                    }
                    connection.transport.seal(&mut report, rtp::RTCP_CLEAR)?;
                    connection.udp.send(&report).await?;
                    limiter.force(now, pacer::Kind::Report, limiter.on_wire(report.len()));
                }
            }
            _ = heartbeat.tick() => connection.heartbeat().await?,
            message = receive(&mut connection.ws, &mut connection.seq) => {
                // Not heartbeat acks, nor the keys in the ready message and
                // session description.
                if let Some((op, data)) = connection.handle(message?).await?
                    && !matches!(op, 2 | 4 | 6)
                {
                    let mut text = data.to_string();
                    text.truncate(400);
                    tracing::debug!(op, data = %text, "the stream server said");
                    // Session update: a viewer can't decode the codec, or the
                    // one who couldn't left.
                    if op == 14
                        && let Some(codec) = data["video_codec"].as_str().and_then(Codec::from_name)
                    {
                        if offered.contains(&codec) && !withdrawn.contains(&codec) {
                            tracing::info!(?codec, "Discord switched the stream's codec");
                            *lock(&shared.chosen) = Some(codec);
                            shared.keyframe.store(true, Ordering::Relaxed);
                        } else {
                            tracing::warn!(?codec, "Discord asked for a codec Yapfast didn't offer");
                        }
                    }
                    // Tell whoever connected about the video again, in case
                    // the server doesn't pass it on to late viewers.
                    if op == 11 {
                        send_json(&mut connection.ws, 12, attributes(ssrc, video, rtx, size, fps, capped)).await?;
                        send_json(&mut connection.ws, 5, json!({"speaking": SOUNDSHARE, "delay": 0, "ssrc": ssrc})).await?;
                    }
                }
            }
            Some(sound) = sounds.recv() => {
                let Some(payload) = connection.dave.encrypt_opus(&sound.opus) else {
                    continue;
                };
                let mut packet =
                    rtp::opus_packet(ssrc, sound_sequence, sound.timestamp, sound_packets == 0, &payload);
                sound_sequence = sound_sequence.wrapping_add(1);
                sound_packets = sound_packets.wrapping_add(1);
                sound_octets = sound_octets.wrapping_add(payload.len() as u32);
                let clear = rtp::rtp_clear_len(&packet).unwrap_or(packet.len());
                connection.transport.seal(&mut packet, clear)?;
                connection.udp.send(&packet).await?;
                // The sound never waits: it's counted, and the video waits
                // the longer.
                let now = std::time::Instant::now();
                limiter.force(now, pacer::Kind::Sound, limiter.on_wire(packet.len()));
                sent.sounds += 1;
                if sound_reported.elapsed() >= SENDER_REPORTS {
                    sound_reported = Instant::now();
                    let mut report = rtp::sender_report(
                        ssrc,
                        std::time::SystemTime::now(),
                        sound.timestamp,
                        sound_packets,
                        sound_octets,
                    );
                    connection.transport.seal(&mut report, rtp::RTCP_CLEAR)?;
                    connection.udp.send(&report).await?;
                    limiter.force(now, pacer::Kind::Report, limiter.on_wire(report.len()));
                }
            }
            received = connection.udp.recv(&mut datagram) => {
                let packet = &datagram[..received?];
                if !rtp::is_rtcp(packet) {
                    sent.rtp_back += 1;
                    continue;
                }
                let Some(rtcp) = connection.transport.open(packet, rtp::RTCP_CLEAR) else {
                    sent.unopened += 1;
                    continue;
                };
                for (kind, said) in rtp::rtcp_kinds(&rtcp) {
                    *sent.rtcp.entry(kind.clone()).or_default() += 1;
                    if !said.is_empty() {
                        sent.said.insert(kind, said);
                    }
                }
                if rtp::wants_keyframe(&rtcp) {
                    sent.requests += 1;
                    shared.keyframe_requests.fetch_add(1, Ordering::Relaxed);
                    shared.keyframe.store(true, Ordering::Relaxed);
                }
                if let Some(estimate) = rtp::bandwidth_estimate(&rtcp, video)
                    && let Some(reason) = pacing.set_estimate(estimate)
                {
                    retarget(shared, &pacing, &limiter, fps, reason, None);
                }
                if let Some(reception) = rtp::reception(&rtcp, video) {
                    let now = std::time::Instant::now();
                    let trip = echoed
                        .iter()
                        .find(|&&(middle, _)| reception.last_report != 0 && middle == reception.last_report)
                        .and_then(|&(_, sent)| rtp::round_trip(sent, now, reception.delay));
                    if let Some(trip) = trip {
                        sent.round_trip_ms = Some(trip.as_millis() as u32);
                    }
                    if let Some(reason) = pacing.report(now, trip, &reception) {
                        retarget(shared, &pacing, &limiter, fps, reason, Some((trip, reception.fraction_lost)));
                    }
                }
                // Resent as the limiter lets them, before the frames' next.
                let now = std::time::Instant::now();
                for lost in rtp::nacked(&rtcp, video) {
                    if resends.iter().any(|&(waiting, _)| waiting == lost) {
                        continue;
                    }
                    if resends.len() >= RESENDS {
                        sent.too_late += 1;
                        continue;
                    }
                    resends.push_back((lost, now));
                }
            }
            _ = stats.tick() => {
                let now = std::time::Instant::now();
                if limiter.settle(now, queued) {
                    tracing::debug!(trim = limiter.trim(), "the encoder's target comes back after it overshot");
                }
                // The overhead measured moves the encoder's target, both
                // ways, at once.
                publish(shared, &pacing, &limiter, fps);
                let rates = limiter.rates(now);
                shared.wire.store(rates.last_second, Ordering::Relaxed);
                shared.peak_100ms.store(rates.peak_100ms, Ordering::Relaxed);
                shared.peak_second.store(rates.peak_second, Ordering::Relaxed);
            }
            _ = log.tick() => {
                let now = std::time::Instant::now();
                let target_kbps = pacing.target() / 1000;
                let encoder_kbps = shared.target.load(Ordering::Relaxed) / 1000;
                let sending_kbps = pacing.sending(now).map(|sending| sending / 1000);
                let received_kbps = pacing.received().map(|received| received / 1000);
                let rates = limiter.rates(now);
                tracing::debug!(
                    ?sent,
                    packets,
                    octets,
                    target_kbps,
                    encoder_kbps,
                    sending_kbps,
                    received_kbps,
                    ceiling_kbps = capped / 1000,
                    wire_kbps = rates.last_second / 1000,
                    peak_100ms_kbps = rates.peak_100ms / 1000,
                    peak_second_kbps = rates.peak_second / 1000,
                    queued_ms = limiter.queued_for(queued).as_millis() as u64,
                    per_packet = limiter.per_packet(now),
                    other_kbps = limiter.other(now) / 1000,
                    trim = limiter.trim(),
                    "stream sent"
                );
                sent = Sent::default();
            }
        }
    }
}

/// How long until the next packet may go, or None with nothing to send: a
/// resend asked for, within its share of the ceiling, or the next of the
/// frames' packets, once the limiter holds half its depth or all that's
/// queued, so a busy stream wakes every few milliseconds rather than for
/// each packet.
fn next_send(
    limiter: &mut Limiter,
    pending: &VecDeque<Vec<u8>>,
    queued: u64,
    resends: &VecDeque<(u16, std::time::Instant)>,
    history: &rtp::History,
) -> Option<Duration> {
    let now = std::time::Instant::now();
    let resend = resends.front().map(|&(lost, _)| {
        // One gone from the history is given up as the branch wakes.
        history.get(lost).map_or(Duration::ZERO, |original| {
            let bits = limiter.on_wire(original.len() + 2 + rtp::SEALING) * 8;
            limiter.resend_wait(now, bits)
        })
    });
    let frames = pending.front().map(|front| {
        let first = limiter.on_wire(front.len() + rtp::SEALING) * 8;
        let batch = queued.min(limiter.depth() / 2).max(first);
        limiter.video_wait(now, batch)
    });
    match (resend, frames) {
        (Some(resend), Some(frames)) => Some(resend.min(frames)),
        (resend, frames) => resend.or(frames),
    }
}

/// Takes the ceiling for frames `height` lines tall, `fps` a second, at
/// once: the limiter's rate and thresholds, the congestion controller's
/// cap, the encoder's target, and the history's room for about a second of
/// packets. Returns the cap op 12 announces after.
fn recap(
    height: u32,
    fps: u32,
    shared: &Shared,
    limiter: &mut Limiter,
    pacing: &mut Controller,
    history: &mut rtp::History,
) -> u32 {
    let wire = shared.wire_ceiling(height, fps);
    limiter.set_ceiling(std::time::Instant::now(), wire, shared.buffer(height));
    shared.ceiling_bps.store(wire, Ordering::Relaxed);
    let overshoot = limiter.overshoot().as_micros() as u64;
    shared.overshoot.store(overshoot, Ordering::Relaxed);
    match pacing.set_cap(shared.encoder_cap(height, fps)) {
        Some(reason) => retarget(shared, pacing, limiter, fps, reason, None),
        None => publish(shared, pacing, limiter, fps),
    }
    history.fit(wire);
    shared.cap(height)
}

/// Hands the encoder its target: the congestion controller's, within what
/// the ceiling leaves the video after the overhead measured lately
/// ([`Limiter::budget`]), less what the encoder overshot
/// ([`Limiter::trim`]).
fn publish(shared: &Shared, pacing: &Controller, limiter: &Limiter, fps: u32) {
    let now = std::time::Instant::now();
    let target = pacing.target().min(limiter.budget(now, fps)) * limiter.trim() / 1000;
    shared
        .target
        .store(target.max(LEAST_TARGET), Ordering::Relaxed);
}

/// Hands the encoder the controller's new target, and logs why it changed,
/// with what the receiver report that changed it said: the round trip it
/// measured, and the packets lost out of 256.
fn retarget(
    shared: &Shared,
    pacing: &Controller,
    limiter: &Limiter,
    fps: u32,
    reason: congestion::Reason,
    report: Option<(Option<Duration>, u8)>,
) {
    publish(shared, pacing, limiter, fps);
    tracing::debug!(
        kbps = pacing.target() / 1000,
        encoder_kbps = shared.target.load(Ordering::Relaxed) / 1000,
        %reason,
        round_trip_ms = report.and_then(|(trip, _)| trip).map(|trip| trip.as_millis() as u64),
        lowest_ms = pacing.baseline().map(|trip| trip.as_millis() as u64),
        lost = report.map(|(_, lost)| lost),
        "changing the stream's bitrate"
    );
}

/// What a streamer sent and heard back since it last logged.
#[derive(Debug, Default)]
struct Sent {
    frames: u32,
    keyframes: u32,
    /// Frames dropped whole: the limiter couldn't send them in time.
    dropped: u32,
    /// Opus frames of the stream's sound.
    sounds: u32,
    /// RTCP by packet type and feedback format, as `type/format`.
    rtcp: std::collections::BTreeMap<String, u32>,
    /// What the last of each kind said.
    said: std::collections::BTreeMap<String, String>,
    /// Keyframe requests among them.
    requests: u32,
    /// Packets resent when a viewer reported them lost, and those given up:
    /// gone from the history, asked for too long ago, or past the most
    /// waiting.
    resent: u32,
    too_late: u32,
    /// RTCP that wouldn't decrypt.
    unopened: u32,
    /// RTP, which a streamer isn't sent.
    rtp_back: u32,
    /// The last round trip a receiver report measured.
    round_trip_ms: Option<u32>,
}

/// What the server said about the streamer's video.
#[derive(Default)]
struct Streamer {
    video: Option<u32>,
    rtx: Option<u32>,
    codec: Option<Codec>,
}

impl Streamer {
    fn learn(&mut self, op: u64, data: &Value) {
        match op {
            // Video: the streamer's SSRCs. Discord's server names the RTX
            // SSRC in the stream that has the video's SSRC, not beside the
            // video's as a sender does, and one it doesn't name is the SSRC
            // after the video's, as call_video.rs takes it.
            12 => {
                let ssrc = |value: &Value| {
                    value
                        .as_u64()
                        .filter(|&ssrc| ssrc != 0)
                        .map(|ssrc| ssrc as u32)
                };
                let Some(video) = ssrc(&data["video_ssrc"]) else {
                    return;
                };
                let stream = data["streams"].as_array().and_then(|streams| {
                    streams
                        .iter()
                        .find(|stream| stream["ssrc"].as_u64() == Some(u64::from(video)))
                });
                self.video = Some(video);
                self.rtx = stream
                    .and_then(|stream| ssrc(&stream["rtx_ssrc"]))
                    .or_else(|| ssrc(&data["rtx_ssrc"]))
                    .or(Some(video.wrapping_add(1)));
            }
            // Session update: the codec may change.
            14 => {
                if let Some(codec) = data["video_codec"].as_str().and_then(Codec::from_name) {
                    self.codec = Some(codec);
                }
            }
            _ => {}
        }
    }

    /// The codec of a packet: the one its payload type names, or, for one
    /// from the streamer's video SSRC whose payload type names none, the
    /// codec its frames are put together in, `assembling`, or else what
    /// Discord chose. Such a packet never starts the frames over: the
    /// codec Discord last named can be out of date.
    fn codec(&self, packet: &rtp::Packet, assembling: Option<Codec>) -> Option<Codec> {
        if Some(packet.ssrc) == self.rtx {
            return None;
        }
        Codec::from_payload_type(packet.payload_type).or_else(|| {
            (Some(packet.ssrc) == self.video)
                .then_some(assembling.or(self.codec))
                .flatten()
        })
    }
}

/// What a viewer's connection has received, logged while no picture
/// reaches the decoder so a stuck stream shows where it stops.
#[derive(Debug, Default)]
struct Received {
    packets: u64,
    rtcp: u64,
    unopened: u64,
    unparsed: u64,
    /// Packets of a payload type that isn't the streamer's video.
    other: u64,
    video: u64,
    frames: u64,
    unreadable: u64,
    decoded: u64,
    /// Opus frames of the stream's sound, and those that wouldn't decrypt.
    sounds: u64,
    unheard: u64,
    /// Packets on the RTX SSRC: resent video, and padding.
    resent: u64,
    padding: u64,
    /// Video packets that came out of order, those that came after they
    /// were asked for again, those that came too late or twice, and those
    /// never seen.
    reordered: u64,
    recovered: u64,
    late: u64,
    given_up: u64,
    /// Packets asked for again (NACK), and keyframes asked for (PLI).
    asked: u64,
    keyframe_requests: u64,
    /// Frames dropped because the decoder was behind.
    behind: u64,
    payload_types: std::collections::BTreeSet<u8>,
    ssrcs: std::collections::BTreeSet<u32>,
}

/// One SSRC's video on its way to frames.
struct Video {
    reorder: rtp::Reorder,
    assembler: Assembler,
    /// For receiver reports: the counts at the last one.
    reported: (u32, u32),
    orientation: Orientation,
}

impl Video {
    fn new(codec: Codec) -> Self {
        Self {
            reorder: rtp::Reorder::default(),
            assembler: Assembler::new(codec),
            reported: (0, 0),
            orientation: Orientation::default(),
        }
    }

    fn push(&mut self, packet: &rtp::Packet<'_>) -> rtp::Pushed {
        // CVO normally rides on the last packet of a frame. Absence means
        // keep the previous orientation; an explicit zero resets it.
        if let Some(orientation) = packet.orientation {
            self.orientation = orientation;
        }
        self.assembler.push(packet)
    }
}

/// Takes a video packet, or a resend of one, into its SSRC's buffers, to
/// be put together into frames once those before it are in.
fn take_video(
    incoming: &mut HashMap<u32, Video>,
    streamer: &mut Streamer,
    packet: &rtp::Packet,
    received: &mut Received,
) {
    let resent = streamer.rtx == Some(packet.ssrc);
    let held = if resent {
        received.resent += 1;
        match streamer
            .video
            .and_then(|video| rtp::Held::unwrap_rtx(packet, video))
        {
            Some(held) => held,
            None => {
                received.padding += 1;
                return;
            }
        }
    } else {
        rtp::Held::new(packet)
    };
    let assembling = incoming
        .get(&held.ssrc)
        .map(|video| video.assembler.codec());
    let Some(codec) = streamer.codec(&held.packet(), assembling) else {
        received.other += 1;
        return;
    };
    received.video += 1;
    streamer.video.get_or_insert(held.ssrc);
    // Packets can name any SSRC: past a few, a new one isn't given buffers
    // of its own.
    if incoming.len() >= MAX_SSRCS && !incoming.contains_key(&held.ssrc) {
        received.other += 1;
        return;
    }
    let video = incoming
        .entry(held.ssrc)
        .or_insert_with(|| Video::new(codec));
    // The streamer switched codecs.
    if video.assembler.codec() != codec {
        video.assembler = Assembler::new(codec);
    }
    match video.reorder.insert(held, std::time::Instant::now()) {
        rtp::Arrival::New if !resent => {}
        rtp::Arrival::New | rtp::Arrival::Recovered if resent => received.recovered += 1,
        rtp::Arrival::New | rtp::Arrival::Recovered => received.reordered += 1,
        rtp::Arrival::Late => received.late += 1,
    }
}

async fn receive_stream(
    server: &StreamServer,
    user: u64,
    channel: u64,
    streamer_id: u64,
    shared: &Shared,
    // Where whole video frames and the sound's Opus frames go.
    (frames, sounds): (
        channel::SyncSender<ReceivedFrame>,
        channel::SyncSender<Vec<u8>>,
    ),
    mut stopped: oneshot::Receiver<()>,
) -> Result<()> {
    let mut connection = open(server, user, channel, Role::Receive, shared).await?;
    let mut streamer = Streamer {
        codec: connection.codec,
        ..Streamer::default()
    };
    for (op, data) in std::mem::take(&mut connection.early) {
        streamer.learn(op, &data);
    }
    // Media sink wants: the server forwards video at the quality a client
    // asks for, here everything at full quality.
    send_json(&mut connection.ws, 15, json!({"any": 100})).await?;
    *lock(&shared.link) = Link::Connected;
    // Whether op 15 last told Discord the video is wanted.
    let mut wanted = true;

    let mut incoming: HashMap<u32, Video> = HashMap::new();
    let mut asked = Instant::now() - KEYFRAME_REQUESTS;
    let interval = connection.interval;
    let mut heartbeat = tokio::time::interval_at(Instant::now() + interval, interval);
    let mut keyframes = tokio::time::interval(KEYFRAME_REQUESTS);
    // Gives up on missing packets and asks for them again even while
    // nothing arrives.
    let mut repairs = tokio::time::interval(Duration::from_millis(20));
    let mut reports = tokio::time::interval(Duration::from_secs(1));
    // The streamer's last sender report, as its NTP time's middle and when
    // it came, which receiver reports echo for the round trip.
    let mut last_report: Option<(u32, std::time::Instant)> = None;
    let mut datagram = vec![0u8; 2048];
    let mut received = Received::default();
    let mut log = tokio::time::interval_at(Instant::now() + LOG_EVERY, LOG_EVERY);
    // Frames handed to the decoder when the log last looked.
    let mut flowing = 0;
    loop {
        let mut lost = false;
        // The video unwanted, or wanted again, once its SSRC is known: what
        // was put together of it goes, and wanted again it starts from a
        // keyframe asked for at once.
        let wants = shared.video_wanted.load(Ordering::Relaxed);
        if wants != wanted
            && let Some(video) = streamer.video
        {
            wanted = wants;
            incoming.clear();
            send_json(&mut connection.ws, 15, sink_wants(video, wanted)).await?;
            if wanted {
                shared.keyframe.store(true, Ordering::Relaxed);
                asked = Instant::now() - KEYFRAME_REQUESTS;
                // Their ticks stopped while it was unwanted: none piled up.
                repairs.reset();
                keyframes.reset();
            }
            tracing::debug!(wanted, "the stream's video");
        }
        tokio::select! {
            _ = &mut stopped => {
                let _ = connection.ws.close(None).await;
                return Ok(());
            }
            _ = heartbeat.tick() => connection.heartbeat().await?,
            message = receive(&mut connection.ws, &mut connection.seq) => {
                if let Some((op, data)) = connection.handle(message?).await? {
                    if !matches!(op, 2 | 4 | 6) {
                        let mut text = data.to_string();
                        text.truncate(400);
                        tracing::debug!(op, data = %text, "the stream server said");
                    }
                    streamer.learn(op, &data);
                }
            }
            // Without the video nothing is put together or asked for again,
            // and the loop sleeps until a packet, or the video's wanted.
            _ = keyframes.tick(), if wanted => {}
            _ = repairs.tick(), if wanted => {}
            () = shared.wants_changed.notified() => {}
            _ = reports.tick() => {
                let Some(media) = streamer.video else { continue };
                let Some(video) = incoming.get_mut(&media) else { continue };
                let Some((highest, lost)) = video.reorder.counts() else { continue };
                let (expected, missed) = (
                    highest.wrapping_sub(video.reported.0),
                    lost.saturating_sub(video.reported.1),
                );
                let fraction = (u64::from(missed) * 256 / u64::from(expected.max(1))).min(255) as u8;
                video.reported = (highest, lost);
                let (middle, delay) = last_report.map_or((0, 0), |(middle, heard)| {
                    (middle, (heard.elapsed().as_secs_f64() * 65536.0) as u32)
                });
                let mut report = rtp::receiver_report(connection.ssrc, media, (highest, lost, fraction), (middle, delay));
                connection.transport.seal(&mut report, rtp::RTCP_CLEAR)?;
                connection.udp.send(&report).await?;
            }
            _ = log.tick() => {
                tracing::debug!(?received, "stream received");
                // Without its video wanted, none reaching the decoder is
                // as asked.
                if received.decoded == flowing && wanted {
                    tracing::info!(
                        ?received,
                        dave = connection.dave.status(),
                        unreadable = connection.dave.unreadable.as_deref(),
                        streamer_video = ?streamer.video,
                        streamer_codec = ?streamer.codec,
                        "no stream frames reached the decoder lately"
                    );
                } else if flowing == 0 {
                    tracing::info!(frames = received.decoded, "stream frames are reaching the decoder");
                }
                flowing = received.decoded;
            }
            length = connection.udp.recv(&mut datagram) => {
                let packet = &datagram[..length?];
                received.packets += 1;
                if rtp::is_rtcp(packet) {
                    received.rtcp += 1;
                    let rtcp = connection.transport.open(packet, rtp::RTCP_CLEAR).unwrap_or_default();
                    for (sender, middle) in rtp::sender_reports(&rtcp) {
                        if Some(sender) == streamer.video {
                            last_report = Some((middle, std::time::Instant::now()));
                        }
                    }
                    continue;
                }
                let Some(clear) = rtp::rtp_clear_len(packet) else {
                    received.unparsed += 1;
                    continue;
                };
                let Some(plain) = connection.transport.open(packet, clear) else {
                    received.unopened += 1;
                    continue;
                };
                let Some(packet) = rtp::parse(&plain) else {
                    received.unparsed += 1;
                    continue;
                };
                received.payload_types.insert(packet.payload_type);
                if received.ssrcs.len() < 64 {
                    received.ssrcs.insert(packet.ssrc);
                }
                if packet.payload_type == rtp::OPUS {
                    received.sounds += 1;
                    shared.sounds.fetch_add(1, Ordering::Relaxed);
                    match connection.dave.decrypt_opus(streamer_id, packet.payload.to_vec()) {
                        // A full queue drops sound the player can't keep up with.
                        Some(sound) => drop(sounds.try_send(sound)),
                        None => received.unheard += 1,
                    }
                    continue;
                }
                // Video still on its way after it was unwanted is dropped.
                if wanted {
                    take_video(&mut incoming, &mut streamer, &packet, &mut received);
                }
            }
        }
        let now = std::time::Instant::now();
        for (&ssrc, video) in &mut incoming {
            let codec = video.assembler.codec();
            let (ready, skipped) = video.reorder.ready(now);
            received.given_up += skipped;
            for held in ready {
                let pushed = video.push(&held.packet());
                lost |= pushed.lost;
                let Some(frame) = pushed.frame else { continue };
                received.frames += 1;
                let Some(data) = connection.dave.decrypt(streamer_id, frame.data) else {
                    received.unreadable += 1;
                    continue;
                };
                received.decoded += 1;
                let frame = ReceivedFrame {
                    codec,
                    data,
                    keyframe: frame.keyframe,
                    orientation: video.orientation,
                };
                if let Err(channel::TrySendError::Full(_)) = frames.try_send(frame) {
                    // The decoder is behind; the frames after this one
                    // need it, so wait for a keyframe.
                    received.behind += 1;
                    video.assembler.wait_for_keyframe();
                    lost = true;
                }
            }
            let missing = video.reorder.to_ask(now);
            if !missing.is_empty() {
                received.asked += missing.len() as u64;
                let mut request = rtp::nack(connection.ssrc, ssrc, &missing);
                connection.transport.seal(&mut request, rtp::RTCP_CLEAR)?;
                connection.udp.send(&request).await?;
            }
        }
        // Ask for a keyframe after a loss, or when the decoder asked, while
        // the video is wanted.
        if (lost || shared.keyframe.load(Ordering::Relaxed))
            && wanted
            && asked.elapsed() >= KEYFRAME_REQUESTS
            && let Some(media) = streamer.video
        {
            shared.keyframe.store(false, Ordering::Relaxed);
            asked = Instant::now();
            received.keyframe_requests += 1;
            let mut request = rtp::picture_loss(connection.ssrc, media);
            connection.transport.seal(&mut request, rtp::RTCP_CLEAR)?;
            connection.udp.send(&request).await?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phone's stream says how each frame turns upright on the frame's
    /// last packet, and only when it has something to say: what it said
    /// last holds for the frames after, until it says otherwise.
    #[test]
    fn mobile_stream_orientation_changes_and_returns_upright() {
        let mut video = Video::new(Codec::H264);
        let mut packetizer = Packetizer::new(7, Codec::H264);
        let mut data = vec![0, 0, 0, 1, 0x65];
        data.resize(3000, 1); // Several RTP packets per frame.
        let turned = |turns, flip| Orientation { turns, flip };
        // Each frame: the byte its last packet carries, whether that packet
        // arrives, and how the frame, or the next, is then shown.
        for (frame, (said, arrives, expected)) in [
            // A stream that never says shows as it comes.
            (None, true, turned(0, false)),
            (Some(1), true, turned(1, false)),
            (None, true, turned(1, false)),
            (Some(3), true, turned(3, false)),
            // Flipped, and with the camera's bit and the reserved ones set,
            // which change nothing shown.
            (Some(0x06), true, turned(2, true)),
            (Some(0xf9), true, turned(1, false)),
            // The packet that says is lost with its frame: the next frames
            // show as before, until one says again.
            (Some(2), false, turned(1, false)),
            (None, true, turned(1, false)),
            (Some(2), true, turned(2, false)),
            // Upright again, said once.
            (Some(0), true, turned(0, false)),
            (None, true, turned(0, false)),
        ]
        .into_iter()
        .enumerate()
        {
            let packets = packetizer.packets(&data, frame as u32 * 3000, true);
            assert!(packets.len() > 2);
            let last = packets.len() - 1;
            for (index, bytes) in packets.iter().enumerate() {
                let mut packet = rtp::parse(bytes).unwrap();
                if index == last {
                    if !arrives {
                        break;
                    }
                    packet.orientation = said.map(Orientation::from_cvo);
                }
                let pushed = video.push(&packet);
                if index < last {
                    assert!(pushed.frame.is_none(), "frame {frame}");
                } else if said.is_some() {
                    assert!(pushed.frame.is_some(), "frame {frame}");
                }
            }
            assert_eq!(video.orientation, expected, "frame {frame}");
        }
        // Said in the middle of a frame, it holds for that frame too.
        let packets = packetizer.packets(&data, 60_000, true);
        for (index, bytes) in packets.iter().enumerate() {
            let mut packet = rtp::parse(bytes).unwrap();
            if index == 1 {
                packet.orientation = Some(Orientation::from_cvo(3));
            }
            if video.push(&packet).frame.is_some() {
                assert_eq!(video.orientation, turned(3, false));
            }
        }
        // Each stream has its own.
        assert_eq!(Video::new(Codec::H264).orientation, Orientation::default());
    }

    /// The person's bitrate limit only lowers Discord's cap for the height,
    /// down to the controller's floor, and op 12 announces what's left.
    #[test]
    fn caps_at_the_limit_within_discords_cap() {
        assert_eq!(cap(1440, None, false), 9_000_000);
        assert_eq!(cap(720, None, false), 3_500_000);
        assert_eq!(cap(1440, Some(5_000), false), 5_000_000);
        assert_eq!(
            cap(720, Some(5_000), false),
            3_500_000,
            "never past Discord's cap"
        );
        assert_eq!(cap(1080, Some(20_000), false), 9_000_000);
        assert_eq!(cap(1080, Some(100), false), congestion::FLOOR as u32);
        assert_eq!(cap(1080, Some(u32::MAX), false), 9_000_000);
        let announced = attributes(1, 2, 3, (3440, 1440), 60, cap(1440, Some(6_000), false));
        assert_eq!(announced["streams"][0]["max_bitrate"], 6_000_000);
        let camera = video_attributes(1, 2, 3, (1280, 720), 30);
        assert_eq!(camera["streams"][0]["max_bitrate"], 3_500_000);
    }

    /// The unlock lets a limit past Discord's documented 9 Mbit/s, up to
    /// 50, at every size, and op 12 announces the limit chosen; Auto stays
    /// Discord's cap. Without it, a limit kept past 9 Mbit/s is 9.
    #[test]
    fn the_unlock_lets_the_limit_past_discords_cap() {
        assert_eq!(cap(1440, Some(20_000), true), 20_000_000);
        assert_eq!(cap(1440, Some(20_000), false), DOCUMENTED_MAX);
        assert_eq!(cap(720, Some(12_000), true), 12_000_000, "any size");
        assert_eq!(cap(1080, Some(60_000), true), UNLOCKED_MAX);
        assert_eq!(cap(1080, Some(u32::MAX), true), UNLOCKED_MAX);
        assert_eq!(cap(1080, Some(1_000), true), 1_000_000, "still lowers");
        assert_eq!(cap(1080, Some(100), true), congestion::FLOOR as u32);
        assert_eq!(cap(1440, None, true), DOCUMENTED_MAX, "Auto is Discord's");
        assert_eq!(cap(720, None, true), 3_500_000);
        let announced = attributes(1, 2, 3, (2560, 1440), 60, cap(1440, Some(30_500), true));
        assert_eq!(announced["streams"][0]["max_bitrate"], 30_500_000);
    }

    /// A limit and its unlock go to the sending thread in one word: the
    /// unlock taken away while live brings the target under Discord's cap
    /// at once, and the limit is kept for when it's unlocked again, which
    /// raises nothing at once.
    #[test]
    fn the_unlock_changed_while_live_moves_the_ceiling() {
        let (frames, _frames) = mpsc::channel(1);
        let (sounds, _sounds) = mpsc::channel(1);
        let sender = StreamSender {
            frames,
            sounds,
            shared: Shared::new((2560, 1440)),
        };
        sender.set_ceiling(Some(20_000), true);
        assert_eq!(sender.shared.ceiling(), (Some(20_000), true));
        assert_eq!(sender.shared.cap(1440), 20_000_000);
        assert_eq!(sender.target(), 2_500_000, "a ceiling, not a target");
        sender.shared.target.store(18_000_000, Ordering::Relaxed);
        sender.set_ceiling(Some(20_000), false);
        assert_eq!(sender.shared.ceiling(), (Some(20_000), false));
        // Discord's cap, which is the video's, as for Auto.
        assert_eq!(sender.target(), u64::from(DOCUMENTED_MAX));
        sender.set_ceiling(Some(20_000), true);
        assert_eq!(sender.target(), u64::from(DOCUMENTED_MAX), "no jump");
        sender.set_ceiling(Some(u32::MAX), true);
        assert_eq!(sender.shared.cap(1440), UNLOCKED_MAX);
        sender.set_ceiling(None, true);
        assert_eq!(sender.shared.ceiling(), (None, true));
    }

    /// 1440p is capped as 1080p is, at every frame rate, and op 12 names
    /// its size as fixed, as for the others.
    #[test]
    fn caps_1440p_as_1080p_at_every_frame_rate() {
        for fps in [15, 30, 60] {
            let announced = attributes(1, 2, 3, (2560, 1440), fps, cap(1440, None, false));
            let stream = &announced["streams"][0];
            assert_eq!(stream["max_bitrate"], 9_000_000, "{fps} fps");
            assert_eq!(stream["max_framerate"], fps);
            assert_eq!(
                stream["max_resolution"],
                json!({"type": "fixed", "width": 2560, "height": 1440})
            );
        }
        // An ultrawide's 1440p, 2560 by 1070, is still above 720 lines.
        assert_eq!(cap(1070, None, false), 9_000_000);
        assert_eq!(cap(1440, Some(9_000), false), 9_000_000, "the highest step");
    }

    /// A limit set while live lowers the target the encoder follows at
    /// once; taken away, it leaves the target for the controller to raise.
    #[test]
    fn a_limit_lowers_the_target_at_once() {
        let (frames, _frames) = mpsc::channel(1);
        let (sounds, _sounds) = mpsc::channel(1);
        let sender = StreamSender {
            frames,
            sounds,
            shared: Shared::new((1920, 1080)),
        };
        assert_eq!(sender.target(), 2_500_000);
        sender.set_ceiling(Some(2_000), false);
        // What 2 Mbit/s on the wire leaves the video.
        let video = pacer::encoder_ceiling(2_000_000, 0);
        assert!((1_500_000..1_800_000).contains(&video), "{video}");
        assert_eq!(
            (sender.target(), sender.shared.ceiling()),
            (video, (Some(2_000), false))
        );
        sender.set_ceiling(None, false);
        assert_eq!(
            (sender.target(), sender.shared.ceiling()),
            (video, (None, false))
        );
    }

    /// A limit the person chose is the ceiling on the wire: op 12 announces
    /// it as it is, and the encoder aims under what it leaves the video
    /// once the packets' overhead, the sound and the limiter's margins are
    /// taken, at every frame rate; Discord's observed 16 Mbit/s leaves about
    /// 13.5. Discord's own cap, at Auto or past a locked limit, is the
    /// video's, as its client sends it, and the wire takes what that needs,
    /// under the 16 Mbit/s Discord was seen to take.
    #[test]
    fn the_encoder_aims_under_what_the_ceiling_leaves() {
        let ceiling = cap(1080, Some(16_000), true);
        assert_eq!(ceiling, OBSERVED_MAX);
        let announced = attributes(1, 2, 3, (1920, 1080), 60, ceiling);
        assert_eq!(
            announced["streams"][0]["max_bitrate"], 16_000_000,
            "the ceiling, no more"
        );
        for fps in [15, 30, 60] {
            let video = encoder_cap(1080, Some(16_000), true, fps);
            assert!(
                (13_300_000..14_000_000).contains(&video),
                "{video} at {fps} fps"
            );
            assert_eq!(video, pacer::encoder_ceiling(16_000_000, fps));
        }
        assert!(
            encoder_cap(1080, Some(16_000), true, 60) < encoder_cap(1080, Some(16_000), true, 15)
        );
        assert_eq!(wire_ceiling(1080, Some(16_000), true, 60), 16_000_000);
        assert!(
            encoder_cap(1080, Some(9_000), false, 30) < 7_800_000,
            "a limit at Discord's cap"
        );
        for (height, max_kbps, unlocked) in [
            (720, None, false),
            (1080, None, true),
            (1080, Some(20_000), false),
        ] {
            for fps in [15, 30, 60] {
                let cap = cap(height, max_kbps, unlocked);
                assert_eq!(encoder_cap(height, max_kbps, unlocked, fps), u64::from(cap));
                let wire = wire_ceiling(height, max_kbps, unlocked, fps);
                assert_eq!(wire, pacer::wire_for(cap.into(), fps));
                assert!(
                    wire > u64::from(cap) * 110 / 100 && wire < u64::from(cap) * 125 / 100,
                    "{wire}"
                );
                assert!(wire < OBSERVED_MAX.into());
            }
        }
        assert_eq!(encoder_buffer(1080, Some(16_000), true), STRICT_BUFFER);
        assert_eq!(encoder_buffer(1080, Some(9_000), true), GENTLE_BUFFER);
        assert_eq!(encoder_buffer(1080, None, true), GENTLE_BUFFER);
    }

    /// Op 12 as Discord's server sends it to a viewer: the RTX SSRC only in
    /// the stream with the video's SSRC.
    fn video_of(video: u32, rtx: u32) -> Value {
        json!({"user_id": "2", "audio_ssrc": video - 1, "video_ssrc": video, "streams": [{
            "ssrc": video, "rtx_ssrc": rtx, "rid": "100", "quality": 100, "active": true,
        }]})
    }

    fn packet(ssrc: u32, payload_type: u8) -> rtp::Packet<'static> {
        rtp::Packet {
            payload_type,
            marker: false,
            sequence: 0,
            timestamp: 0,
            ssrc,
            payload: &[],
            orientation: None,
        }
    }

    #[test]
    fn learns_the_streamers_video() {
        let mut streamer = Streamer::default();
        streamer.learn(12, &video_of(11, 15));
        assert_eq!((streamer.video, streamer.rtx), (Some(11), Some(15)));
        streamer.learn(14, &json!({"video_codec": "H264"}));
        assert_eq!(
            streamer.codec(&packet(11, rtp::VP8), None),
            Some(Codec::Vp8),
            "by payload type"
        );
        assert_eq!(
            streamer.codec(&packet(11, 96), None),
            Some(Codec::H264),
            "by Discord's choice"
        );
        assert_eq!(
            streamer.codec(&packet(11, 96), Some(Codec::Av1)),
            Some(Codec::Av1),
            "as the frames on the way"
        );
        assert_eq!(
            streamer.codec(&packet(11, rtp::VP8), Some(Codec::Av1)),
            Some(Codec::Vp8),
            "a switch"
        );
        assert_eq!(
            streamer.codec(&packet(15, rtp::VP8_RTX), None),
            None,
            "retransmissions aren't frames"
        );
        assert_eq!(streamer.codec(&packet(30, 96), Some(Codec::Av1)), None);

        // A stream that names no RTX SSRC resends on the one after its
        // video's; one sent as a sender sends it names it beside the video.
        let mut streamer = Streamer::default();
        streamer.learn(
            12,
            &json!({"user_id": "2", "audio_ssrc": 20, "video_ssrc": 21,
                    "streams": [{"ssrc": 21, "rid": "100", "active": true}]}),
        );
        assert_eq!((streamer.video, streamer.rtx), (Some(21), Some(22)));
        streamer.learn(
            12,
            &json!({"user_id": "2", "audio_ssrc": 30, "video_ssrc": 31, "rtx_ssrc": 35}),
        );
        assert_eq!((streamer.video, streamer.rtx), (Some(31), Some(35)));
        // Another layer's stream doesn't give the video's.
        streamer.learn(
            12,
            &json!({"user_id": "2", "audio_ssrc": 40, "video_ssrc": 41, "streams": [
                {"ssrc": 51, "rtx_ssrc": 52, "rid": "50"},
                {"ssrc": 41, "rtx_ssrc": 44, "rid": "100"},
            ]}),
        );
        assert_eq!((streamer.video, streamer.rtx), (Some(41), Some(44)));
        // No video: what was learned stands.
        streamer.learn(
            12,
            &json!({"user_id": "3", "audio_ssrc": 60, "video_ssrc": 0}),
        );
        assert_eq!((streamer.video, streamer.rtx), (Some(41), Some(44)));
    }

    /// The video's packets in, frames out, as the connection's loop passes
    /// them on.
    fn assemble(incoming: &mut HashMap<u32, Video>) -> Vec<rtp::Assembled> {
        let mut frames = vec![];
        for video in incoming.values_mut() {
            let (ready, _) = video.reorder.ready(std::time::Instant::now());
            for held in ready {
                frames.extend(video.push(&held.packet()).frame);
            }
        }
        frames
    }

    /// A packet lost on the way comes again on the RTX SSRC that op 12's
    /// stream names, and completes its frame.
    #[test]
    fn takes_resends_from_the_rtx_ssrc_the_stream_names() {
        let mut streamer = Streamer::default();
        streamer.learn(12, &video_of(11, 12));
        let mut packetizer = Packetizer::new(11, Codec::H264);
        let mut keyframe = vec![0, 0, 0, 1, 0x65];
        keyframe.resize(3000, 1);
        let sent = packetizer.packets(&keyframe, 0, true);
        assert!(sent.len() > 2);
        let (mut incoming, mut received) = (HashMap::new(), Received::default());
        let mut frames = vec![];
        for (index, bytes) in sent.iter().enumerate() {
            if index != 1 {
                take_video(
                    &mut incoming,
                    &mut streamer,
                    &rtp::parse(bytes).unwrap(),
                    &mut received,
                );
                frames.extend(assemble(&mut incoming));
            }
        }
        assert!(frames.is_empty(), "waiting for the lost packet");
        let again = packetizer.retransmit(&sent[1], 12).unwrap();
        take_video(
            &mut incoming,
            &mut streamer,
            &rtp::parse(&again).unwrap(),
            &mut received,
        );
        frames.extend(assemble(&mut incoming));
        assert_eq!(
            frames
                .iter()
                .map(|frame| (frame.keyframe, &frame.data))
                .collect::<Vec<_>>(),
            [(true, &keyframe)]
        );
        assert_eq!(
            (received.resent, received.recovered, received.other),
            (1, 1, 0)
        );
    }

    /// AV1 frames go on whole past packets on the video's SSRC whose
    /// payload type names no codec, while op 4 named another codec, as in
    /// a user's log: those packets don't start the frames over from the
    /// next keyframe.
    #[test]
    fn an_unknown_payload_type_keeps_the_frames_going() {
        let mut streamer = Streamer {
            codec: Some(Codec::H264),
            ..Streamer::default()
        };
        streamer.learn(12, &video_of(11, 12));
        let mut packetizer = Packetizer::new(11, Codec::Av1);
        let keyframe = [&[1 << 3 | 2, 3, 1, 2, 3][..], &[6 << 3 | 2, 4, 9, 9, 9, 9]].concat();
        let delta = vec![6 << 3 | 2, 4, 8, 8, 8, 8];
        let (mut incoming, mut received) = (HashMap::new(), Received::default());
        let mut sequence = 0u16;
        let mut keyframes = vec![];
        for (index, frame) in [&keyframe, &delta, &delta].into_iter().enumerate() {
            let timestamp = index as u32 * 3000;
            let sent = packetizer.packets(frame, timestamp, index == 0);
            let stray = rtp::Packet {
                sequence,
                timestamp,
                ..packet(11, 96)
            };
            sequence += 1;
            let mut packets = vec![stray];
            for bytes in &sent {
                packets.push(rtp::Packet {
                    sequence,
                    ..rtp::parse(bytes).unwrap()
                });
                sequence += 1;
            }
            for packet in packets {
                take_video(&mut incoming, &mut streamer, &packet, &mut received);
                keyframes.extend(assemble(&mut incoming).iter().map(|frame| frame.keyframe));
            }
        }
        assert_eq!(keyframes, [true, false, false]);
        assert_eq!(incoming[&11].assembler.codec(), Codec::Av1);
    }

    /// Unwanted, a stream's video is asked for at 0 by its SSRC, and
    /// everything else, its sound among it, still in full; wanted again,
    /// at 100.
    #[test]
    fn wants_a_streams_video_or_only_its_sound() {
        assert_eq!(sink_wants(5001, false), json!({"any": 100, "5001": 0}));
        assert_eq!(sink_wants(5001, true), json!({"any": 100, "5001": 100}));
    }
}
