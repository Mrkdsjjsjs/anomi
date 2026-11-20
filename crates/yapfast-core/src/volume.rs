//! How loud each person sounds in calls, and their stream's sound, as the
//! gain they play at: 1.0 as they come, 0.0 silent, up to [`MAX`]. People are
//! named as their network does, by Discord user id or Matrix user id. The UI
//! remembers the choices and sets them here; the calls' audio reads them as
//! it plays. Its sliders show them on Discord's scale; see [`from_slider`].
//! The microphone's volume and the speakers' apply to every call on top:
//! everyone plays at their own volume times the speakers'. Muting someone's
//! soundboard here silences the sounds they play, not their voice. Someone's
//! voice can also be cleaned here before it's mixed ([`Cleaning`]), in
//! Discord calls, whose audio Yapfast mixes itself, and so can the sound of
//! someone's stream watched, apart from their voice
//! ([`set_stream_cleaning`]), which the watching worker plays.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        LazyLock, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::noise::Suppressor;

/// Twice as loud, as Discord allows.
pub const MAX: f32 = 2.0;

/// The gain of a slider at `value`, from 0 to 200, as Discord's volume
/// sliders map it: below 100 on a power curve, `(value / 100)^2.8`, so 50 is
/// about -17 dB; above it 6 dB more for each 100, so 200 is about twice as
/// loud. A linear slider made everything loud: 40 on Discord's is 0.077, not
/// 0.4.
pub fn from_slider(value: f32) -> f32 {
    let share = value.max(0.0) / 100.0;
    if share < 1.0 {
        share.powf(2.8)
    } else {
        10f32.powf((share - 1.0) * 6.0 / 20.0)
    }
}

/// The slider value that plays at `gain`; see [`from_slider`].
pub fn to_slider(gain: f32) -> f32 {
    if gain <= 0.0 {
        0.0
    } else if gain < 1.0 {
        gain.powf(1.0 / 2.8) * 100.0
    } else {
        (20.0 * gain.log10() / 6.0 + 1.0) * 100.0
    }
}

/// A person's voice, or their stream's sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Voice,
    Stream,
}

impl Kind {
    /// The gain unless one is chosen: voices as they come, and a stream's
    /// sound at Discord's default of 0.18, about 54 on the slider, since
    /// it's played as loud as the streamer's computer makes it.
    pub fn default_gain(self) -> f32 {
        match self {
            Kind::Voice => 1.0,
            Kind::Stream => 0.18,
        }
    }
}

/// The volumes chosen, voices' then streams', by person; looked up by
/// borrowed id, as the calls' audio does for every frame.
static VOLUMES: LazyLock<[RwLock<HashMap<String, f32>>; 2]> = LazyLock::new(Default::default);
/// Those muted for this account alone, as Discord's local mute does.
static MUTED: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(RwLock::default);
/// Those whose soundboard sounds this account doesn't hear.
static SOUNDBOARD_MUTED: LazyLock<RwLock<HashSet<String>>> = LazyLock::new(RwLock::default);
/// Those whose voice is cleaned before it's mixed, and how; no one else's is.
static CLEANING: LazyLock<RwLock<HashMap<String, Cleaning>>> = LazyLock::new(RwLock::default);
/// Those whose stream's sound is cleaned before it plays, and how, by the
/// streamer: the same choices as a voice's, kept apart from them.
static STREAM_CLEANING: LazyLock<RwLock<HashMap<String, Cleaning>>> =
    LazyLock::new(RwLock::default);
static CHANGED: LazyLock<watch::Sender<u64>> = LazyLock::new(|| watch::channel(0).0);

/// The microphone's and the speakers' gains in every call, as f32 bits.
static MICROPHONE: AtomicU32 = AtomicU32::new(1f32.to_bits());
static SPEAKERS: AtomicU32 = AtomicU32::new(1f32.to_bits());

fn volumes(kind: Kind) -> &'static RwLock<HashMap<String, f32>> {
    &VOLUMES[kind as usize]
}

/// Sets how loud `person` sounds; the default gain forgets the choice.
pub fn set(kind: Kind, person: &str, volume: f32) {
    let volume = volume.clamp(0.0, MAX);
    {
        let mut volumes = volumes(kind).write().unwrap();
        if (volume - kind.default_gain()).abs() < f32::EPSILON {
            volumes.remove(person);
        } else {
            volumes.insert(person.to_owned(), volume);
        }
    }
    CHANGED.send_modify(|changes| *changes += 1);
}

/// How loud `person` sounds, as chosen.
pub fn get(kind: Kind, person: &str) -> f32 {
    volumes(kind)
        .read()
        .unwrap()
        .get(person)
        .copied()
        .unwrap_or(kind.default_gain())
}

/// Mutes `person`'s voice for this account alone, keeping their volume
/// for when they're unmuted, or unmutes them.
pub fn mute(person: &str, muted: bool) {
    let changed = {
        let mut set = MUTED.write().unwrap();
        if muted {
            set.insert(person.to_owned())
        } else {
            set.remove(person)
        }
    };
    if changed {
        CHANGED.send_modify(|changes| *changes += 1);
    }
}

/// Whether `person` is muted for this account alone.
pub fn muted(person: &str) -> bool {
    MUTED.read().unwrap().contains(person)
}

/// Mutes the soundboard sounds `person` plays, for this account alone, or
/// unmutes them; the sounds they play from then on.
pub fn mute_soundboard(person: &str, muted: bool) {
    let mut set = SOUNDBOARD_MUTED.write().unwrap();
    if muted {
        set.insert(person.to_owned());
    } else {
        set.remove(person);
    }
}

/// Whether `person`'s soundboard sounds are muted for this account.
pub fn soundboard_muted(person: &str) -> bool {
    SOUNDBOARD_MUTED.read().unwrap().contains(person)
}

/// How loud `person` plays: as chosen, times the speakers' volume, or
/// silent while muted here.
pub fn heard(kind: Kind, person: &str) -> f32 {
    if kind == Kind::Voice && muted(person) {
        0.0
    } else {
        get(kind, person) * speakers()
    }
}

/// What's done to someone's voice here before it's mixed, as their menu in
/// a call and the card over it ask, or to their stream's sound before it
/// plays, as its tile's menu asks: nothing unless chosen. Each costs
/// nothing while they're silent, muted here or deafened to. It travels to a
/// watching worker as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Cleaning {
    /// What takes the noise out of their voice: RNNoise, the light one,
    /// which plays them 10 ms later, or DeepFilterNet, the strong one, on a
    /// thread of its own (`their_noise`), which plays them 50 ms later and
    /// costs about seven times the processor; `Off` for neither. Either way
    /// they play in mono.
    pub noise: Suppressor,
    /// How much of their noise it takes out, in percent: 0 leaves their
    /// voice as it comes, [`FULL_STRENGTH`] takes out all it's let to
    /// ([`Cleaning::noise_limit`]).
    pub noise_strength: u8,
    /// They're turned down while what comes from them is only this
    /// account's own voice, back from their speakers (`echo_gate`)...
    pub own_echo: bool,
    /// ...by this many dB: [`ECHO_DEPTH`] unless chosen, up to
    /// [`MOST_ECHO_DEPTH`]; 0 leaves them as they come.
    pub echo_depth: u8,
}

/// Noise suppression's strength unless chosen: all of it, as before it
/// could be chosen.
pub const FULL_STRENGTH: u8 = 100;
/// How far our echo from someone is turned down unless chosen, in dB, and
/// the most it can be.
pub const ECHO_DEPTH: u8 = 20;
pub const MOST_ECHO_DEPTH: u8 = 40;
/// How far each suppressor turns someone's noise down at full strength, in
/// dB. RNNoise 30, as Medium does the microphone's: what it mixes back of
/// their voice keeps the ends of words. DeepFilterNet 40, past Medium; with
/// no limit at all it takes giggles and breathy laughs for noise.
const LIGHT_LIMIT: f32 = 30.0;
const STRONG_LIMIT: f32 = 40.0;

impl Cleaning {
    /// Nothing done, each strength as it is unless chosen.
    pub const NOTHING: Self = Self {
        noise: Suppressor::Off,
        noise_strength: FULL_STRENGTH,
        own_echo: false,
        echo_depth: ECHO_DEPTH,
    };

    /// Whether anything is done.
    pub fn any(self) -> bool {
        self.denoises() || self.removes_echo()
    }

    /// Whether a suppressor takes any of their noise out.
    pub fn denoises(self) -> bool {
        self.noise != Suppressor::Off && self.noise_strength > 0
    }

    /// Whether they're turned down at all while only our echo comes.
    pub fn removes_echo(self) -> bool {
        self.own_echo && self.echo_depth > 0
    }

    /// How far `suppressor` turns their noise down at this strength, in
    /// dB: the strength's share of its limit at full strength, so the
    /// slider moves evenly in what's heard. Both keep to it by mixing their
    /// voice as it came back in, 10^(-dB/20) of it: RNNoise into its
    /// output, as the microphone's does, DeepFilterNet into its spectrum,
    /// as its own attenuation limit. Cheap, and with no artefact of its
    /// own: at 0 dB it is their voice as it came.
    pub fn noise_limit(self, suppressor: Suppressor) -> f32 {
        let full = match suppressor {
            Suppressor::DeepFilterNet => STRONG_LIMIT,
            Suppressor::RNNoise | Suppressor::Off => LIGHT_LIMIT,
        };
        full * f32::from(self.noise_strength.min(FULL_STRENGTH)) / f32::from(FULL_STRENGTH)
    }
}

impl Default for Cleaning {
    fn default() -> Self {
        Self::NOTHING
    }
}

/// Sets what's done to `person`'s voice before it's mixed. They're
/// forgotten once nothing is chosen, strengths included; a strength chosen
/// while nothing is done is kept, at no cost.
pub fn set_cleaning(person: &str, cleaning: Cleaning) {
    put(&CLEANING, person, cleaning);
}

/// What's done to `person`'s voice before it's mixed.
pub fn cleaning(person: &str) -> Cleaning {
    read(&CLEANING, person)
}

/// Sets what's done to the sound of `person`'s stream before it plays,
/// apart from their voice: noise suppression is made for speech, and a
/// stream's music or game sound suffers from it. Forgotten as a voice's is.
pub fn set_stream_cleaning(person: &str, cleaning: Cleaning) {
    put(&STREAM_CLEANING, person, cleaning);
}

/// What's done to the sound of `person`'s stream before it plays.
pub fn stream_cleaning(person: &str) -> Cleaning {
    read(&STREAM_CLEANING, person)
}

/// Keeps `cleaning` for `person` in `kept`, forgetting them once nothing
/// is chosen, and says a call should copy it again.
fn put(kept: &RwLock<HashMap<String, Cleaning>>, person: &str, cleaning: Cleaning) {
    let changed = {
        let mut people = kept
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cleaning == Cleaning::NOTHING {
            people.remove(person).is_some()
        } else {
            people.insert(person.to_owned(), cleaning) != Some(cleaning)
        }
    };
    if changed {
        CHANGED.send_modify(|changes| *changes += 1);
    }
}

fn read(kept: &RwLock<HashMap<String, Cleaning>>, person: &str) -> Cleaning {
    kept.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(person)
        .copied()
        .unwrap_or_default()
}

/// Each person whose voice doesn't play as it comes: the gain it plays at
/// before the speakers' volume, 0.0 while muted here, and how it's
/// cleaned. Anyone else plays at 1.0, uncleaned. A Discord call keeps a
/// copy, which it looks up without locks, and copies again when
/// [`changes`] says one changed. A person may come more than once, the
/// same each time.
#[cfg(feature = "discord")]
pub(crate) fn each_voice(mut each: impl FnMut(&str, f32, Cleaning)) {
    let volumes = volumes(Kind::Voice)
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let muted = MUTED
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cleaning = CLEANING
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for person in volumes.keys().chain(muted.iter()).chain(cleaning.keys()) {
        let gain = if muted.contains(person) {
            0.0
        } else {
            volumes
                .get(person)
                .copied()
                .unwrap_or(Kind::Voice.default_gain())
        };
        let cleaned = cleaning.get(person).copied().unwrap_or_default();
        each(person.as_str(), gain, cleaned);
    }
}

/// Sets how loud the microphone sends in every call, from 0.0 to [`MAX`].
pub fn set_microphone(gain: f32) {
    MICROPHONE.store(gain.clamp(0.0, MAX).to_bits(), Ordering::Relaxed);
}

/// How loud the microphone sends; 1.0, as it comes, unless set.
pub fn microphone() -> f32 {
    f32::from_bits(MICROPHONE.load(Ordering::Relaxed))
}

/// Sets how loud every call plays, from 0.0 to [`MAX`].
pub fn set_speakers(gain: f32) {
    let gain = gain.clamp(0.0, MAX).to_bits();
    if SPEAKERS.swap(gain, Ordering::Relaxed) != gain {
        CHANGED.send_modify(|changes| *changes += 1);
    }
}

/// How loud every call plays; 1.0 unless set.
pub fn speakers() -> f32 {
    f32::from_bits(SPEAKERS.load(Ordering::Relaxed))
}

/// Changes whenever a volume, a mute or someone's [`Cleaning`] does, their
/// stream's too.
pub fn changes() -> watch::Receiver<u64> {
    CHANGED.subscribe()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held by the tests that set the speakers' volume or depend on it.
    static SPEAKERS_SET: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn remembers_each_persons_voice_and_stream() {
        let mut changes = changes();
        set(Kind::Voice, "volume-test-1", 0.5);
        set(Kind::Stream, "volume-test-1", 3.0);
        assert!(changes.has_changed().unwrap());
        changes.mark_unchanged();
        assert_eq!(get(Kind::Voice, "volume-test-1"), 0.5);
        assert_eq!(
            get(Kind::Stream, "volume-test-1"),
            MAX,
            "at most twice as loud"
        );
        assert_eq!(
            get(Kind::Voice, "volume-test-2"),
            1.0,
            "unchanged by default"
        );
        set(Kind::Voice, "volume-test-1", 1.0);
        assert_eq!(get(Kind::Voice, "volume-test-1"), 1.0);
        assert!(changes.has_changed().unwrap());
    }

    #[test]
    fn a_local_mute_silences_the_voice_and_keeps_its_volume() {
        let _speakers = SPEAKERS_SET.lock().unwrap();
        set(Kind::Voice, "volume-test-3", 0.5);
        let changes = changes();
        mute("volume-test-3", true);
        assert!(changes.has_changed().unwrap());
        assert!(muted("volume-test-3"));
        assert_eq!(heard(Kind::Voice, "volume-test-3"), 0.0);
        assert_eq!(
            heard(Kind::Stream, "volume-test-3"),
            Kind::Stream.default_gain(),
            "their stream plays"
        );
        assert_eq!(
            get(Kind::Voice, "volume-test-3"),
            0.5,
            "the slider keeps it"
        );
        mute("volume-test-3", false);
        assert_eq!(heard(Kind::Voice, "volume-test-3"), 0.5);
    }

    #[test]
    fn muting_the_soundboard_leaves_the_voice() {
        mute_soundboard("volume-test-6", true);
        assert!(soundboard_muted("volume-test-6"));
        assert!(!muted("volume-test-6"), "their voice plays");
        mute_soundboard("volume-test-6", false);
        assert!(!soundboard_muted("volume-test-6"));
    }

    #[test]
    fn cleaning_is_kept_by_person_until_nothing_is_done() {
        let person = "volume-test-7";
        assert_eq!(cleaning(person), Cleaning::NOTHING, "nothing by default");
        let mut changes = changes();
        let both = Cleaning {
            noise: Suppressor::DeepFilterNet,
            own_echo: true,
            ..Cleaning::NOTHING
        };
        set_cleaning(person, both);
        assert!(changes.has_changed().unwrap(), "a call copies it at once");
        changes.mark_unchanged();
        assert_eq!(cleaning(person), both);
        let noise = Cleaning {
            noise: Suppressor::RNNoise,
            noise_strength: 40,
            ..Cleaning::NOTHING
        };
        set_cleaning(person, noise);
        assert_eq!(cleaning(person), noise);
        // A strength with nothing on is kept for when it is.
        let kept = Cleaning {
            noise: Suppressor::Off,
            ..noise
        };
        set_cleaning(person, kept);
        assert!(!cleaning(person).any());
        assert_eq!(cleaning(person).noise_strength, 40);
        set_cleaning(person, Cleaning::NOTHING);
        assert!(changes.has_changed().unwrap());
        assert_eq!(cleaning(person), Cleaning::NOTHING, "forgotten");
        assert_eq!(get(Kind::Voice, person), 1.0, "the volume stays");
    }

    #[test]
    fn a_streams_cleaning_is_kept_apart_from_its_streamers_voice() {
        let person = "volume-test-12";
        let mut changes = changes();
        let noise = Cleaning {
            noise: Suppressor::RNNoise,
            ..Cleaning::NOTHING
        };
        set_stream_cleaning(person, noise);
        assert!(changes.has_changed().unwrap(), "the watch hears of it");
        changes.mark_unchanged();
        assert_eq!(stream_cleaning(person), noise);
        assert_eq!(
            cleaning(person),
            Cleaning::NOTHING,
            "their voice as it comes"
        );
        set_stream_cleaning(person, Cleaning::NOTHING);
        assert_eq!(stream_cleaning(person), Cleaning::NOTHING, "forgotten");
        // It travels to the worker as it is.
        let sent = serde_json::to_string(&noise).unwrap();
        assert_eq!(serde_json::from_str::<Cleaning>(&sent).unwrap(), noise);
    }

    #[test]
    fn a_strength_of_none_does_nothing_and_full_is_each_suppressors_limit() {
        let rnnoise = Cleaning {
            noise: Suppressor::RNNoise,
            ..Cleaning::NOTHING
        };
        assert!(rnnoise.denoises());
        assert_eq!(rnnoise.noise_limit(Suppressor::RNNoise), 30.0, "as before");
        assert_eq!(rnnoise.noise_limit(Suppressor::DeepFilterNet), 40.0);
        let half = Cleaning {
            noise_strength: 50,
            ..rnnoise
        };
        assert_eq!(half.noise_limit(Suppressor::RNNoise), 15.0);
        let none = Cleaning {
            noise_strength: 0,
            ..rnnoise
        };
        assert!(!none.denoises() && !none.any(), "their voice as it comes");
        let echo = Cleaning {
            own_echo: true,
            ..Cleaning::NOTHING
        };
        assert!(echo.removes_echo());
        assert_eq!(echo.echo_depth, 20, "as before");
        let flat = Cleaning {
            echo_depth: 0,
            ..echo
        };
        assert!(!flat.removes_echo() && !flat.any());
    }

    #[cfg(feature = "discord")]
    #[test]
    fn a_call_copies_everyone_who_plays_other_than_as_they_come() {
        set(Kind::Voice, "volume-test-8", 0.5);
        mute("volume-test-9", true);
        set(Kind::Voice, "volume-test-9", 1.5);
        let noise = Cleaning {
            noise: Suppressor::RNNoise,
            ..Cleaning::NOTHING
        };
        set_cleaning("volume-test-10", noise);
        let mut copied = HashMap::new();
        each_voice(|person, gain, cleaning| {
            copied.insert(person.to_owned(), (gain, cleaning));
        });
        assert_eq!(copied["volume-test-8"], (0.5, Cleaning::default()));
        assert_eq!(
            copied["volume-test-9"],
            (0.0, Cleaning::default()),
            "muted here"
        );
        assert_eq!(
            copied["volume-test-10"],
            (1.0, noise),
            "as they come, cleaned"
        );
        assert!(!copied.contains_key("volume-test-11"));
        mute("volume-test-9", false);
        set_cleaning("volume-test-10", Cleaning::default());
    }

    #[test]
    fn the_speakers_volume_scales_everyone() {
        let _speakers = SPEAKERS_SET.lock().unwrap();
        set(Kind::Voice, "volume-test-5", 0.5);
        let changes = changes();
        set_speakers(1.5);
        assert!(changes.has_changed().unwrap(), "calls play it at once");
        assert_eq!(heard(Kind::Voice, "volume-test-5"), 0.75);
        assert_eq!(
            heard(Kind::Stream, "volume-test-5"),
            Kind::Stream.default_gain() * 1.5,
            "streams too"
        );
        assert_eq!(get(Kind::Voice, "volume-test-5"), 0.5, "their own stays");
        set_speakers(1.0);
    }

    #[test]
    fn sliders_follow_discords_curve() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        assert_eq!(from_slider(0.0), 0.0, "silent");
        assert!(close(from_slider(100.0), 1.0), "as they come");
        assert!(close(from_slider(50.0), 0.143), "about -17 dB");
        assert!(close(from_slider(40.0), 0.077));
        assert!(close(from_slider(200.0), 1.995), "6 dB louder");
        for value in [1.0, 18.0, 40.0, 99.0, 100.0, 150.0, 200.0] {
            assert!(close(to_slider(from_slider(value)), value), "{value}");
        }
        // A stream's default shows at about 54, as on Discord.
        assert_eq!(to_slider(Kind::Stream.default_gain()).round(), 54.0);
        assert_eq!(get(Kind::Stream, "volume-test-4"), 0.18);
    }
}
