//! Carol on a laptop's speakers, sending back what Alice says: what the
//! call tells its listener it sent, for the echo gate, and the echo coming
//! back.

use super::*;

use std::{
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

/// Carol counts as heard with a sample this loud.
const LOUD: i16 = 3_000;

/// A call's audio: what it said it sent each tick, and what came from Carol.
struct Echoed {
    carol: u64,
    /// Ticks that sent the microphone, and those that sent none.
    sent: AtomicU32,
    unsent: AtomicU32,
    /// Frames sent that weren't 20 ms of stereo with both channels alike.
    malformed: AtomicU32,
    /// When the microphone was first sent, and Carol first heard loud.
    first_sent: Mutex<Option<Instant>>,
    first_heard: Mutex<Option<Instant>>,
    /// The loudest sample heard from her.
    loudest: Mutex<i16>,
}

impl Echoed {
    fn new(carol: u64) -> Self {
        Self {
            carol,
            sent: AtomicU32::new(0),
            unsent: AtomicU32::new(0),
            malformed: AtomicU32::new(0),
            first_sent: Mutex::new(None),
            first_heard: Mutex::new(None),
            loudest: Mutex::new(0),
        }
    }
}

impl Listener for Echoed {
    fn tick(&self, voices: &[(u64, &[i16])]) {
        for (user, samples) in voices {
            if *user != self.carol {
                continue;
            }
            let loudest = samples
                .iter()
                .map(|sample| sample.saturating_abs())
                .max()
                .unwrap_or(0);
            let mut heard = self.loudest.lock().unwrap();
            *heard = (*heard).max(loudest);
            let mut first = self.first_heard.lock().unwrap();
            if loudest > LOUD && first.is_none() {
                *first = Some(Instant::now());
            }
        }
    }

    fn sent(&self, frame: Option<&[f32]>) {
        let Some(frame) = frame else {
            self.unsent.fetch_add(1, Ordering::Relaxed);
            return;
        };
        self.sent.fetch_add(1, Ordering::Relaxed);
        let mut first = self.first_sent.lock().unwrap();
        if first.is_none() {
            *first = Some(Instant::now());
        }
        let stereo = frame.len() == yapfast_discord::voice::FRAME * 2
            && frame.chunks_exact(2).all(|pair| pair[0] == pair[1]);
        if !stereo {
            self.malformed.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn disconnected(&self, reason: &str) {
        panic!("voice disconnected: {reason}");
    }
}

/// Says `frames` 20 ms frames of a tone into the microphone, in time.
async fn speak(microphone: &yapfast_discord::voice::Microphone, frames: usize) {
    for frame in 0..frames {
        let tone: Vec<f32> = (0..yapfast_discord::voice::FRAME)
            .map(|at| {
                let t = (frame * yapfast_discord::voice::FRAME + at) as f32 / 48_000.0;
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.4
            })
            .collect();
        microphone.push(&tone);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The call tells its listener each tick what it sent, nothing until the
/// microphone plays; Carol, asked to, sends Alice's voice back to her call
/// as her own, 300 ms later, and stops when asked.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts testing/fake_discord.ts with bun; run scripts/test-fake-discord"]
async fn carol_sends_alices_voice_back() {
    let fixture = Fixture::start("carol_sends_alices_voice_back").await;
    let carol: u64 = fixture.snapshot.voice[&fixture.lounge]
        .iter()
        .find(|member| member.name.starts_with("Carol"))
        .map(|member| member.user_id.parse().unwrap())
        .expect("Carol sits in the Lounge");
    let assigned = fixture
        .discord
        .join(&fixture.guild, &fixture.lounge)
        .await
        .unwrap();
    let echoed = Arc::new(Echoed::new(carol));
    let mut call = timeout(
        WAIT,
        Call::connect(&assigned, &fixture.guild, &fixture.lounge, echoed.clone()),
    )
    .await
    .expect("timed out connecting to voice")
    .unwrap();
    post_control(fixture.server, "/_control/carol-echo?delay=300", "");
    until("the call said it sent nothing", || {
        echoed.unsent.load(Ordering::Relaxed) >= 10
    })
    .await;
    assert_eq!(echoed.sent.load(Ordering::Relaxed), 0);

    let (microphone, input) = yapfast_discord::voice::live_microphone();
    let track = call.play(input);
    speak(&microphone, 25).await;
    track.pause().unwrap();
    until("Carol sent Alice's voice back", || {
        echoed.first_heard.lock().unwrap().is_some()
    })
    .await;
    let sent = echoed.sent.load(Ordering::Relaxed);
    assert!(sent >= 20, "the call said it sent {sent} frames of 25");
    assert_eq!(echoed.malformed.load(Ordering::Relaxed), 0);
    let first_sent = echoed.first_sent.lock().unwrap().expect("a frame went");
    let first_heard = echoed.first_heard.lock().unwrap().expect("Carol came");
    let late = first_heard.duration_since(first_sent);
    assert!(
        late >= Duration::from_millis(250),
        "Carol came back after only {late:?}"
    );

    // Stopped, she sends nothing back.
    post_control(fixture.server, "/_control/carol-echo?off", "");
    tokio::time::sleep(Duration::from_millis(600)).await;
    *echoed.loudest.lock().unwrap() = 0;
    track.play().unwrap();
    speak(&microphone, 25).await;
    track.pause().unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let loudest = *echoed.loudest.lock().unwrap();
    assert!(
        loudest < LOUD,
        "Carol was heard at {loudest} after she stopped"
    );
}
