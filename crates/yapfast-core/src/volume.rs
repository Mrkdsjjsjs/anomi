//! How loud each person sounds in calls, and their stream's sound, as the
//! gain they play at: 1.0 as they come, 0.0 silent, up to [`MAX`]. People are
//! named as their network does, by Discord user id or Matrix user id. The UI
//! remembers the choices and sets them here; the calls' audio reads them as
//! it plays. Its sliders show them on Discord's scale; see [`from_slider`].
//! The microphone's volume and the speakers' apply to every call on top:
//! everyone plays at their own volume times the speakers'. Muting someone's
//! soundboard here silences the sounds they play, not their voice. Someone's
//! voice can also be cleaned here before it's mixed ([`Cleaning`]), in
//! Discord calls, whose audio Yapfast mixes itself.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        LazyLock, RwLock,
        atomic::{AtomicU32, Ordering},
    },
};

use tokio::sync::watch;

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

/// What's done to someone's voice here before it's mixed, as the boxes in
/// their menu in a call ask: nothing unless chosen. Each costs nothing while
/// they're silent, muted here or deafened to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cleaning {
    /// RNNoise takes the noise out of their voice, which then plays 10 ms
    /// later, in mono.
    pub noise: bool,
    /// They're turned down while what comes from them is only this
    /// account's own voice, back from their speakers (`echo_gate`).
    pub own_echo: bool,
}

impl Cleaning {
    /// Whether anything is done.
    pub fn any(self) -> bool {
        self.noise || self.own_echo
    }
}

/// Sets what's done to `person`'s voice before it's mixed; nothing done
/// forgets them.
pub fn set_cleaning(person: &str, cleaning: Cleaning) {
    let changed = {
        let mut people = CLEANING
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if cleaning.any() {
            people.insert(person.to_owned(), cleaning) != Some(cleaning)
        } else {
            people.remove(person).is_some()
        }
    };
    if changed {
        CHANGED.send_modify(|changes| *changes += 1);
    }
}

/// What's done to `person`'s voice before it's mixed.
pub fn cleaning(person: &str) -> Cleaning {
    CLEANING
        .read()
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

/// Changes whenever a volume, a mute or someone's [`Cleaning`] does.
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
        assert_eq!(cleaning(person), Cleaning::default(), "nothing by default");
        let mut changes = changes();
        let both = Cleaning {
            noise: true,
            own_echo: true,
        };
        set_cleaning(person, both);
        assert!(changes.has_changed().unwrap(), "a call copies it at once");
        changes.mark_unchanged();
        assert_eq!(cleaning(person), both);
        let noise = Cleaning {
            noise: true,
            own_echo: false,
        };
        set_cleaning(person, noise);
        assert_eq!(cleaning(person), noise);
        set_cleaning(person, Cleaning::default());
        assert!(changes.has_changed().unwrap());
        assert!(!cleaning(person).any(), "forgotten");
        assert_eq!(get(Kind::Voice, person), 1.0, "the volume stays");
    }

    #[cfg(feature = "discord")]
    #[test]
    fn a_call_copies_everyone_who_plays_other_than_as_they_come() {
        set(Kind::Voice, "volume-test-8", 0.5);
        mute("volume-test-9", true);
        set(Kind::Voice, "volume-test-9", 1.5);
        let noise = Cleaning {
            noise: true,
            own_echo: false,
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
