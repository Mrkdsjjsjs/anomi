//! The echo gate on made-up voices, whose parts are known, and over a real
//! call's two sides, for the ears.

use super::*;

const RATE: f32 = 48_000.0;
/// Seeds for our voice, theirs and their microphone's noise.
const OURS: u64 = 0x9e37_79b9_7f4a_7c15;
const THEIRS: u64 = 0xd1b5_4a32_d192_ed03;
const NOISE: u64 = 0x2545_f491_4f6c_dd1d;

#[test]
fn turns_our_echo_down_150_300_and_600_ms_late() {
    let ours = speech(OURS, 20.0, 120.0, -22.0);
    for late in [150.0, 300.0, 600.0] {
        let heard = mixed(&echo(&ours, |_| late, -12.0), &noise(ours.len(), -65.0));
        let mut gate = EchoGate::new();
        let out = run(&mut gate, &ours, &heard);
        let second = RATE as usize;
        assert!(
            out[..second] == heard[..second],
            "it changed what came {late} ms late while it was learning"
        );
        let found = found_ms(&gate);
        assert!(
            found.is_some_and(|found| (found - late).abs() <= 40.0),
            "it found {found:?} ms for {late} ms"
        );
        let down = change(&heard, &out, 8.0, 20.0);
        assert!(
            down <= -14.0,
            "our echo {late} ms late went only {down:.1} dB down"
        );
    }
}

#[test]
fn lets_them_through_when_they_talk_over_our_echo() {
    let ours = speech(OURS, 20.0, 120.0, -22.0);
    let voice = placed(&speech(THEIRS, 12.0, 210.0, -20.0), 8.0, ours.len());
    let room = mixed(&echo(&ours, |_| 300.0, -12.0), &noise(ours.len(), -65.0));
    let heard = mixed(&room, &voice);
    let mut gate = EchoGate::new();
    let out = run(&mut gate, &ours, &heard);
    assert!(gate.delay().is_some(), "it never found our echo");
    let share = kept(&voice, &heard, &out, 8.0, 20.0);
    assert!(
        share >= 0.8,
        "only {:.0}% of their voice came through",
        share * 100.0
    );
}

#[test]
fn lets_them_through_in_full_when_they_speak_alone() {
    let length = (20.0 * RATE) as usize;
    let ours = mixed(
        &placed(&speech(OURS, 10.0, 120.0, -22.0), 0.0, length),
        &placed(&speech(OURS ^ 1, 3.0, 120.0, -22.0), 14.0, length),
    );
    let voice = mixed(
        &placed(&speech(THEIRS, 2.5, 210.0, -24.0), 11.0, length),
        &placed(&speech(THEIRS ^ 1, 1.5, 210.0, -24.0), 18.5, length),
    );
    let room = mixed(&echo(&ours, |_| 300.0, -12.0), &noise(length, -65.0));
    let heard = mixed(&room, &voice);
    let mut gate = EchoGate::new();
    let out = run(&mut gate, &ours, &heard);
    assert!(gate.delay().is_some(), "it never found our echo");
    for (from, to) in [(11.0, 13.5), (18.5, 20.0)] {
        let share = kept(&voice, &heard, &out, from, to);
        assert!(
            share >= 0.99,
            "only {:.1}% of them from {from} s came through",
            share * 100.0
        );
    }
    let down = change(&heard, &out, 15.0, 17.0);
    assert!(
        down <= -12.0,
        "our echo between their turns went only {down:.1} dB down"
    );
}

#[test]
fn never_turns_down_a_voice_that_isnt_our_echo() {
    // As on headphones: they talk, over us and not, and our voice never
    // comes back.
    let ours = speech(OURS, 20.0, 120.0, -22.0);
    let voice = placed(&speech(THEIRS, 19.3, 210.0, -24.0), 0.7, ours.len());
    let heard = mixed(&voice, &noise(ours.len(), -65.0));
    let mut gate = EchoGate::new();
    let out = run(&mut gate, &ours, &heard);
    assert_eq!(gate.delay(), None);
    assert!(out == heard, "it turned down a voice that wasn't our echo");
}

#[test]
fn follows_a_delay_that_drifts() {
    let ours = speech(OURS, 25.0, 120.0, -22.0);
    // 200 ms at first, then 10 ms later every 2.5 s as jitter buffers grow:
    // 280 ms from 20 s on.
    let drifting = |seconds: f32| 200.0 + 10.0 * (seconds / 2.5).floor().min(8.0);
    let room = mixed(&echo(&ours, drifting, -12.0), &noise(ours.len(), -65.0));
    let mut gate = EchoGate::new();
    let out = run(&mut gate, &ours, &room);
    let found = found_ms(&gate);
    assert!(
        found.is_some_and(|found| (found - 280.0).abs() <= 40.0),
        "it found {found:?} ms for 280 ms"
    );
    let down = change(&room, &out, 10.0, 25.0);
    assert!(
        down <= -13.0,
        "our drifting echo went only {down:.1} dB down"
    );
}

#[test]
fn does_nothing_in_silence_or_without_us() {
    let mut gate = EchoGate::new();
    let quiet = [0.0f32; FRAME];
    let mut heard = [0.0f32; FRAME];
    for tick in 0..400 {
        gate.sent((tick >= 200).then_some(&quiet[..]));
        gate.heard(&mut heard);
    }
    assert!(heard.iter().all(|sample| *sample == 0.0));
    assert_eq!(gate.delay(), None);
    assert!(!gate.turned_down());
    // Our voice to someone silent: no echo to find.
    let ours = speech(OURS, 6.0, 120.0, -22.0);
    let silent = vec![0.0; ours.len()];
    let out = run(&mut gate, &ours, &silent);
    assert!(out == silent);
    assert_eq!(gate.delay(), None);
    // Their voice with nothing of ours to look for in it: as it came.
    let voice = speech(THEIRS, 3.0, 210.0, -24.0);
    let mut out = voice.clone();
    let mut fresh = EchoGate::new();
    for frame in out.chunks_exact_mut(FRAME) {
        fresh.heard(frame);
    }
    assert!(out == voice);
}

#[test]
fn turns_down_over_60_ms_and_back_up_over_5() {
    let gate = EchoGate::new();
    assert!((gate.fall.powf(FALL) - gate.ducked).abs() < 1e-3);
    assert!((gate.ducked * gate.rise.powf(RISE) - 1.0).abs() < 1e-3);
    assert_eq!(AROUND[0].0, -(SOONER as isize));
    assert_eq!(AROUND[AROUND.len() - 1].0, LATER as isize);
}

#[cfg(feature = "discord")]
#[test]
fn turns_our_echo_down_through_opus_both_ways() {
    let ours = speech(OURS, 20.0, 120.0, -22.0);
    // Our voice reaches them through Opus, and comes back through it again.
    let there = through_opus(&ours);
    let room = mixed(&echo(&there, |_| 300.0, -12.0), &noise(ours.len(), -65.0));
    let heard = through_opus(&room);
    let mut gate = EchoGate::new();
    let out = run(&mut gate, &ours, &heard);
    let found = found_ms(&gate);
    assert!(
        found.is_some_and(|found| (found - 300.0).abs() <= 40.0),
        "it found {found:?} ms for 300 ms"
    );
    let down = change(&heard, &out, 8.0, 20.0);
    assert!(
        down <= -12.0,
        "our echo through Opus went only {down:.1} dB down"
    );
}

/// Runs the gate over a real call, for the ears: `YAPFAST_ECHO_SENT`, what
/// this account said, and `YAPFAST_ECHO_HEARD`, what came back from someone
/// on speakers, two recordings started together, each a WAV file at 48 kHz
/// (16-bit or float, mixed down to mono) or raw mono 16-bit. What the gate
/// lets through goes to `YAPFAST_ECHO_OUT`, a mono WAV file, and it prints
/// the delay it found and how much it turned down each second. With
/// PipeWire, for one:
///
/// ```text
/// pw-record --target <microphone> sent.wav &
/// pw-record -P '{ stream.capture.sink=true }' --target <speakers> heard.wav
/// YAPFAST_ECHO_SENT=sent.wav YAPFAST_ECHO_HEARD=heard.wav YAPFAST_ECHO_OUT=gated.wav \
///   cargo test --profile fast -p yapfast-core gates_a_recording -- --ignored --nocapture
/// ```
///
/// A raw microphone sends all the time where a call's gate wouldn't: a tick
/// quieter than `YAPFAST_ECHO_OPEN_DB`, -50 dBFS unless set, counts as sent
/// only within 400 ms of a louder one. A silent tick from them counts as
/// none, as a call leaves out someone silent.
#[test]
#[ignore = "needs YAPFAST_ECHO_SENT, YAPFAST_ECHO_HEARD and YAPFAST_ECHO_OUT"]
fn gates_a_recording() {
    let path = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
    let sent = read_sound(&path("YAPFAST_ECHO_SENT"));
    let heard = read_sound(&path("YAPFAST_ECHO_HEARD"));
    let open: f32 = std::env::var("YAPFAST_ECHO_OPEN_DB").map_or(-50.0, |level| {
        level
            .parse()
            .expect("YAPFAST_ECHO_OPEN_DB is a level in dBFS")
    });
    let ticks = sent.len().min(heard.len()) / FRAME;
    let mut out = heard[..ticks * FRAME].to_vec();
    let mut gate = EchoGate::new();
    let mut open_for = 0u32;
    let (mut second, mut whole) = (Counts::default(), Counts::default());
    let frames = sent.chunks_exact(FRAME).zip(out.chunks_exact_mut(FRAME));
    for (tick, (ours, theirs)) in frames.enumerate() {
        open_for = if level(ours) > open {
            20
        } else {
            open_for.saturating_sub(1)
        };
        gate.sent((open_for > 0).then_some(ours));
        if level(theirs) > -90.0 {
            let came = energy(theirs);
            gate.heard(theirs);
            second.add(came, energy(theirs), gate.turned_down());
        }
        if tick % 50 == 49 {
            let delay = match gate.delay() {
                Some(delay) => format!("{} ms", delay.as_millis()),
                None => "not found".to_owned(),
            };
            println!(
                "{:5} s: delay {delay}; {}",
                (tick + 1) / 50,
                second.report()
            );
            whole.merge(second);
            second = Counts::default();
        }
    }
    whole.merge(second);
    println!("in all: {}", whole.report());
    write_sound(&path("YAPFAST_ECHO_OUT"), &out);
}

/// What the gate did over a while: ticks heard and turned down, and the
/// energy that came and that it let through.
#[derive(Clone, Copy, Default)]
struct Counts {
    heard: u32,
    turned_down: u32,
    came: f64,
    through: f64,
}

impl Counts {
    fn add(&mut self, came: f64, through: f64, turned_down: bool) {
        self.heard += 1;
        self.turned_down += u32::from(turned_down);
        self.came += came;
        self.through += through;
    }

    fn merge(&mut self, other: Self) {
        self.heard += other.heard;
        self.turned_down += other.turned_down;
        self.came += other.came;
        self.through += other.through;
    }

    fn report(&self) -> String {
        let down = 10.0 * (self.through.max(1e-30) / self.came.max(1e-30)).log10();
        format!(
            "{} of {} ticks heard turned down, {down:.1} dB in all",
            self.turned_down, self.heard
        )
    }
}

/// Noise that's the same on every run: xorshift.
struct Random(u64);

impl Random {
    /// Uniform from 0 to 1.
    fn uniform(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn between(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.uniform()
    }
}

/// A two-pole resonance, as of a vocal tract.
#[derive(Default)]
struct Resonance {
    input: f32,
    feedback: [f32; 2],
    past: [f32; 2],
}

impl Resonance {
    /// Tunes it to `frequency` Hz, `width` Hz wide.
    fn tune(&mut self, frequency: f32, width: f32) {
        let radius = (-std::f32::consts::PI * width / RATE).exp();
        let angle = std::f32::consts::TAU * frequency / RATE;
        self.input = 1.0 - radius;
        self.feedback = [2.0 * radius * angle.cos(), -radius * radius];
    }

    fn pass(&mut self, sample: f32) -> f32 {
        let out =
            self.input * sample + self.feedback[0] * self.past[0] + self.feedback[1] * self.past[1];
        self.past = [out, self.past[0]];
        out
    }
}

/// `seconds` of someone speaking, at `level` dBFS while they do: words of
/// three to six syllables with pauses between them, each syllable a buzz at
/// about `pitch` Hz through two resonances that move from one to the next,
/// now and then with a hiss.
fn speech(seed: u64, seconds: f32, pitch: f32, level: f32) -> Vec<f32> {
    let mut random = Random(seed);
    let mut sound = vec![0.0; (seconds * RATE) as usize];
    let (mut low, mut high) = (Resonance::default(), Resonance::default());
    let (mut phase, mut last) = (0.0, 0.0);
    let mut at = 0;
    while at < sound.len() {
        let syllables = 3 + (random.uniform() * 4.0) as usize;
        for _ in 0..syllables {
            let length = (random.between(0.12, 0.25) * RATE) as usize;
            low.tune(random.between(300.0, 800.0), 90.0);
            high.tune(random.between(900.0, 2_400.0), 140.0);
            let step = pitch * random.between(0.85, 1.15) / RATE;
            let hiss = if random.uniform() < 0.3 { 0.05 } else { 0.0 };
            for index in 0..length {
                let noise = random.uniform() - 0.5;
                phase += step;
                let pulse = if phase >= 1.0 {
                    phase -= 1.0;
                    1.0
                } else {
                    0.0
                };
                let envelope = (std::f32::consts::PI * index as f32 / length as f32).sin();
                let voiced = low.pass(pulse) + 0.6 * high.pass(pulse);
                if let Some(sample) = sound.get_mut(at + index) {
                    *sample = envelope * (voiced + hiss * (noise - last));
                }
                last = noise;
            }
            at += length + (random.between(0.03, 0.12) * RATE) as usize;
        }
        at += (random.between(0.15, 0.45) * RATE) as usize;
    }
    let (power, count) = sound
        .iter()
        .filter(|sample| **sample != 0.0)
        .fold((0.0, 0u32), |(power, count), sample| {
            (power + f64::from(sample * sample), count + 1)
        });
    let rms = (power / f64::from(count.max(1))).sqrt() as f32;
    let scale = amplitude(level) / rms.max(1e-9);
    for sample in &mut sound {
        *sample *= scale;
    }
    sound
}

/// `length` samples of white noise at `level` dBFS.
fn noise(length: usize, level: f32) -> Vec<f32> {
    let mut random = Random(NOISE);
    // Uniform noise from -0.5 to 0.5 has an RMS of 1/√12.
    let scale = amplitude(level) * 12f32.sqrt();
    (0..length)
        .map(|_| scale * (random.uniform() - 0.5))
        .collect()
}

/// What their microphone picks up of `ours` from their speakers, `gain` dB
/// louder and `delay` ms later, given the time in seconds: their speakers
/// play no bass, and the room adds two reflections.
fn echo(ours: &[f32], delay: impl Fn(f32) -> f32, gain: f32) -> Vec<f32> {
    let gain = amplitude(gain);
    // A one-pole high-pass at 250 Hz.
    let pole = (-std::f32::consts::TAU * 250.0 / RATE).exp();
    let (mut before, mut passed) = (0.0, 0.0);
    let mut echoed = Vec::with_capacity(ours.len());
    for at in 0..ours.len() {
        let late = (delay(at as f32 / RATE) / 1000.0 * RATE) as usize;
        let back = |extra: usize| at.checked_sub(late + extra).map_or(0.0, |from| ours[from]);
        let played = gain * (back(0) + 0.5 * back(480) + 0.25 * back(1_680));
        passed = pole * (passed + played - before);
        before = played;
        echoed.push(passed);
    }
    echoed
}

/// `sound` placed `at` seconds into `length` samples of silence.
fn placed(sound: &[f32], at: f32, length: usize) -> Vec<f32> {
    let mut placed = vec![0.0; length];
    let start = ((at * RATE) as usize).min(length);
    let end = (start + sound.len()).min(length);
    placed[start..end].copy_from_slice(&sound[..end - start]);
    placed
}

/// Two sounds of the same length played together.
fn mixed(one: &[f32], other: &[f32]) -> Vec<f32> {
    one.iter()
        .zip(other)
        .map(|(one, other)| one + other)
        .collect()
}

/// Runs the gate as a call does, tick by tick: what we sent, or nothing on a
/// tick of silence, then what came from them. Returns what it let through.
fn run(gate: &mut EchoGate, ours: &[f32], heard: &[f32]) -> Vec<f32> {
    let mut out = heard.to_vec();
    for (tick, theirs) in out.chunks_exact_mut(FRAME).enumerate() {
        let said = ours.get(tick * FRAME..(tick + 1) * FRAME);
        gate.sent(said.filter(|frame| frame.iter().any(|sample| *sample != 0.0)));
        gate.heard(theirs);
    }
    out
}

/// The delay the gate found, in ms.
fn found_ms(gate: &EchoGate) -> Option<f32> {
    gate.delay().map(|delay| delay.as_millis() as f32)
}

/// How much louder `out` is than `heard` from `from` to `to` seconds, in dB.
fn change(heard: &[f32], out: &[f32], from: f32, to: f32) -> f64 {
    let range = (from * RATE) as usize..(to * RATE) as usize;
    10.0 * (energy(&out[range.clone()]) / energy(&heard[range])).log10()
}

fn energy(sound: &[f32]) -> f64 {
    let energy: f64 = sound.iter().map(|sample| f64::from(*sample).powi(2)).sum();
    energy.max(1e-30)
}

/// The share of `part`'s energy, one of the sounds `heard` is made of, that
/// came through from `from` to `to` seconds. The gate scales all it hears
/// alike, by `out` over `heard`.
fn kept(part: &[f32], heard: &[f32], out: &[f32], from: f32, to: f32) -> f64 {
    let range = (from * RATE) as usize..(to * RATE) as usize;
    let samples = part[range.clone()]
        .iter()
        .zip(&heard[range.clone()])
        .zip(&out[range]);
    let (mut before, mut after) = (0.0, 0.0);
    for ((one, came), went) in samples {
        let scale = if came.abs() > 1e-9 { went / came } else { 1.0 };
        before += f64::from(*one).powi(2);
        after += f64::from(one * scale).powi(2);
    }
    after / f64::max(before, 1e-30)
}

/// A tick's level, in dBFS.
fn level(sound: &[f32]) -> f32 {
    let power = sound.iter().map(|sample| sample * sample).sum::<f32>();
    10.0 * (power / sound.len().max(1) as f32 + 1e-12).log10()
}

/// `sound` encoded with Opus as the call encodes a microphone, at Discord's
/// 64 kbit/s, and decoded.
#[cfg(feature = "discord")]
fn through_opus(sound: &[f32]) -> Vec<f32> {
    use yapfast_discord::opus::{Application, Decoder, Encoder};
    let mut encoder = Encoder::new(1, Application::Voice, 64_000).expect("an Opus encoder");
    let mut decoder = Decoder::new(1).expect("an Opus decoder");
    let (mut packet, mut decoded) = (vec![0; 1_500], vec![0; FRAME]);
    let mut coded = Vec::with_capacity(sound.len());
    for frame in sound.chunks_exact(FRAME) {
        let length = encoder
            .encode_float(frame, &mut packet)
            .expect("Opus encodes");
        let samples = decoder
            .decode(&packet[..length], &mut decoded, false)
            .expect("Opus decodes");
        coded.extend(
            decoded[..samples]
                .iter()
                .map(|sample| f32::from(*sample) / 32_768.0),
        );
    }
    coded
}

/// A recording's samples at 48 kHz, mixed down to mono: a WAV file, 16-bit
/// or float, or raw mono 16-bit without a header.
fn read_sound(path: &str) -> Vec<f32> {
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("can't read {path}: {error}"));
    if bytes.get(..4) != Some(b"RIFF".as_slice()) || bytes.get(8..12) != Some(b"WAVE".as_slice()) {
        return bytes.chunks_exact(2).map(pcm16).collect();
    }
    let (mut format, mut channels, mut bits) = (0u16, 0usize, 0u16);
    let mut at = 12;
    while let Some(header) = bytes.get(at..at + 8) {
        let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let start = at + 8;
        let body = &bytes[start.min(bytes.len())..(start + size).min(bytes.len())];
        match &header[..4] {
            b"fmt " => {
                assert!(body.len() >= 16, "{path} has too short a format");
                let field = |at: usize| u16::from_le_bytes([body[at], body[at + 1]]);
                let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
                assert_eq!(
                    rate, 48_000,
                    "{path} is at {rate} Hz: resample it to 48 kHz first"
                );
                (format, channels, bits) = (field(0), usize::from(field(2)), field(14));
                // WAVE_FORMAT_EXTENSIBLE names its format in its subformat.
                if format == 0xfffe && body.len() >= 26 {
                    format = field(24);
                }
            }
            b"data" => {
                // A recording stopped before its header was finished has no length.
                let body = if size == 0 {
                    &bytes[start.min(bytes.len())..]
                } else {
                    body
                };
                return mixed_down(body, format, channels, bits)
                    .unwrap_or_else(|| panic!("{path} is neither 16-bit nor float"));
            }
            _ => {}
        }
        at = start + size + size % 2;
    }
    panic!("{path} has no sound in it");
}

/// A WAV file's interleaved samples, 16-bit (format 1) or float (3), mixed
/// down to mono.
fn mixed_down(body: &[u8], format: u16, channels: usize, bits: u16) -> Option<Vec<f32>> {
    let sample: fn(&[u8]) -> f32 = match (format, bits) {
        (1, 16) => pcm16,
        (3, 32) => float32,
        _ => return None,
    };
    if channels == 0 {
        return None;
    }
    let width = usize::from(bits / 8);
    let mono = body
        .chunks_exact(channels * width)
        .map(|frame| frame.chunks_exact(width).map(sample).sum::<f32>() / channels as f32);
    Some(mono.collect())
}

fn pcm16(bytes: &[u8]) -> f32 {
    f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0
}

fn float32(bytes: &[u8]) -> f32 {
    f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Writes mono samples at 48 kHz as a 16-bit WAV file.
fn write_sound(path: &str, samples: &[f32]) {
    let size = u32::try_from(samples.len() * 2).expect("a WAV file holds at most 4 GB");
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    // 16 bytes of format: PCM, one channel, 48 kHz, 96 000 bytes a second,
    // two bytes a sample, 16 bits.
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&96_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&size.to_le_bytes());
    for sample in samples {
        let value = (sample * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap_or_else(|error| panic!("can't write {path}: {error}"));
}
