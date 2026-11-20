//! How this device cleans each person's voice in Discord calls, as
//! `layout.json` keeps it: `cleaned_voices`, by user id, Anomi's own key,
//! which the Slint app keeps through its catch-all. Each person's choices
//! leave out what's as it is unless chosen:
//!
//! ```json
//! "cleaned_voices": {
//!   "1016732398987264101": {"noise": "deepfilternet", "noise_strength": 60,
//!                           "echo": true, "echo_depth": 30},
//!   "1016732398987264102": {"noise": "rnnoise"}
//! }
//! ```
//!
//! A stream's sound watched is kept the same way, apart from its streamer's
//! voice, under `cleaned_streams`, by the streamer's user id, from the menu
//! of the stream's tile; it never names a key of the voices':
//!
//! ```json
//! "cleaned_streams": {
//!   "1016732398987264102": {"noise": "rnnoise", "noise_strength": 40}
//! }
//! ```
//!
//! An earlier Anomi kept two sets of user ids instead, `denoised_people` and
//! `echo_removed_people`. [`Saved`] doesn't know them, so they come in its
//! catch-all, from which they're read into the map as the file is
//! ([`Saved::settle`]), as what they did, and never written again.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use yapfast_core::noise::Suppressor;
use yapfast_core::volume::{self, Cleaning};

use super::Saved;

/// The sets an earlier Anomi kept: whose noise RNNoise took out, and whose
/// voice our echo was taken out of.
const DENOISED: &str = "denoised_people";
const ECHO_REMOVED: &str = "echo_removed_people";

/// One person's, as `layout.json` keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanedVoice {
    /// What takes the noise out of their voice: `"rnnoise"`, the light
    /// one, `"deepfilternet"`, the strong one, or `"off"`, left out.
    #[serde(skip_serializing_if = "is_off")]
    pub noise: Suppressor,
    /// How much of their noise it takes out, in percent; all unless set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noise_strength: Option<u8>,
    /// Their voice is turned down while only this account's echo comes
    /// from them, by `echo_depth` dB; 20 unless set.
    #[serde(skip_serializing_if = "is_false")]
    pub echo: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub echo_depth: Option<u8>,
}

impl Default for CleanedVoice {
    fn default() -> Self {
        Self {
            noise: Suppressor::Off,
            noise_strength: None,
            echo: false,
            echo_depth: None,
        }
    }
}

impl CleanedVoice {
    /// As `layout.json` keeps `cleaning`: a strength only where it isn't
    /// the one unless chosen.
    pub fn of(cleaning: Cleaning) -> Self {
        Self {
            noise: cleaning.noise,
            noise_strength: (cleaning.noise_strength != volume::FULL_STRENGTH)
                .then_some(cleaning.noise_strength),
            echo: cleaning.own_echo,
            echo_depth: (cleaning.echo_depth != volume::ECHO_DEPTH).then_some(cleaning.echo_depth),
        }
    }

    /// What the call does to their voice.
    pub fn cleaning(self) -> Cleaning {
        let strength = self.noise_strength.unwrap_or(volume::FULL_STRENGTH);
        let depth = self.echo_depth.unwrap_or(volume::ECHO_DEPTH);
        Cleaning {
            noise: self.noise,
            noise_strength: strength.min(volume::FULL_STRENGTH),
            own_echo: self.echo,
            echo_depth: depth.min(volume::MOST_ECHO_DEPTH),
        }
    }
}

impl Saved {
    /// Reads the two sets an earlier Anomi kept into [`Saved::cleaned_voices`],
    /// as what they did: someone in `denoised_people` has RNNoise take their
    /// noise out in full, someone in `echo_removed_people` our echo taken
    /// out of their voice 20 dB. A suppressor the map already has for them
    /// stays. The sets are never written, so the next write keeps the map
    /// alone; until then a start reads them again, and nothing is lost.
    pub(super) fn settle_cleaning(&mut self) {
        for person in taken(&mut self.other, DENOISED) {
            let voice = self.cleaned_voices.entry(person).or_default();
            if voice.noise == Suppressor::Off {
                voice.noise = Suppressor::RNNoise;
            }
        }
        for person in taken(&mut self.other, ECHO_REMOVED) {
            self.cleaned_voices.entry(person).or_default().echo = true;
        }
    }
}

/// The user ids an earlier Anomi kept as `key`, taken out of the keys
/// `layout.json` has that [`Saved`] doesn't know.
fn taken(other: &mut Map<String, Value>, key: &str) -> Vec<String> {
    let Some(Value::Array(people)) = other.remove(key) else {
        return Vec::new();
    };
    (people.into_iter())
        .filter_map(|person| match person {
            Value::String(person) => Some(person),
            _ => None,
        })
        .collect()
}

fn is_off(suppressor: &Suppressor) -> bool {
    *suppressor == Suppressor::Off
}

fn is_false(on: &bool) -> bool {
    !*on
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::*;
    use crate::layout::Layout;

    /// Each person's cleaning reads and writes in its own shape, a choice
    /// left out where it's as it is unless chosen; keys neither app knows
    /// stay.
    #[test]
    fn each_persons_cleaning_keeps_its_own_shape() {
        let layout = Layout::from_values(
            PathBuf::new(),
            serde_json::from_str(
                r#"{"cleaned_voices": {"7": {"noise": "deepfilternet", "noise_strength": 60,
                    "echo": true, "echo_depth": 30}, "8": {"noise_strength": 250}},
                    "a_newer_key": [1]}"#,
            )
            .unwrap(),
        );
        let seven = Cleaning {
            noise: Suppressor::DeepFilterNet,
            noise_strength: 60,
            own_echo: true,
            echo_depth: 30,
        };
        assert_eq!(layout.cleaned_voices["7"].cleaning(), seven);
        let eight = layout.cleaned_voices["8"].cleaning();
        assert!(!eight.any() && eight.noise_strength == 100, "at most all");
        assert_eq!(CleanedVoice::of(seven), layout.cleaned_voices["7"]);
        assert_eq!(CleanedVoice::of(Cleaning::NOTHING), CleanedVoice::default());
        let file: Value = serde_json::from_slice(&layout.bytes().unwrap()).unwrap();
        let written = json!({"noise": "deepfilternet", "noise_strength": 60,
            "echo": true, "echo_depth": 30});
        assert_eq!(file["cleaned_voices"]["7"], written);
        assert_eq!(file["a_newer_key"], json!([1]), "kept as it came");
        assert!(file.get("cleaned_streams").is_none(), "none written unset");
    }

    /// A stream's sound keeps its own key, in the voices' shape, its
    /// streamer's voice apart; keys neither app knows stay.
    #[test]
    fn each_streams_cleaning_keeps_its_own_key() {
        let mut layout = Layout::from_values(
            PathBuf::new(),
            serde_json::from_str(
                r#"{"cleaned_streams": {"7": {"noise": "rnnoise", "noise_strength": 40}},
                    "cleaned_voices": {"7": {"echo": true}}, "a_newer_key": {"x": 1}}"#,
            )
            .unwrap(),
        );
        let stream = Cleaning {
            noise: Suppressor::RNNoise,
            noise_strength: 40,
            ..Cleaning::NOTHING
        };
        assert_eq!(layout.cleaned_streams["7"].cleaning(), stream);
        assert!(
            !layout.cleaned_voices["7"].cleaning().denoises(),
            "the voice apart"
        );
        layout.edit(|saved| {
            let echo = Cleaning {
                own_echo: true,
                ..Cleaning::NOTHING
            };
            saved
                .cleaned_streams
                .insert("8".into(), CleanedVoice::of(echo));
        });
        let file: Value = serde_json::from_slice(&layout.bytes().unwrap()).unwrap();
        assert_eq!(
            file["cleaned_streams"],
            json!({"7": {"noise": "rnnoise", "noise_strength": 40}, "8": {"echo": true}})
        );
        assert_eq!(file["cleaned_voices"], json!({"7": {"echo": true}}));
        assert_eq!(file["a_newer_key"], json!({"x": 1}), "kept as it came");
    }
}
