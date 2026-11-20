//! The menu of a watched stream's tile, Anomi's own, under the card with its
//! volume (`intent::menus::stream`), and the card's own rows over that
//! volume, as Discord's menu of a stream starts ([`card_rows`]): "Stop
//! Watching", then under a line the box "Mute". Under the card, what cleans
//! the stream's sound, as
//! someone's menu in a call cleans their voice. "Suppress noise" opens the
//! same suppressors, and says under it that they're made for speech, which
//! music and a game's sound come out of damaged; "Remove my echo" turns the
//! stream down while what comes from it is only this account's own voice,
//! back from the streamer's system sound. The rows report the ids of the
//! voice's (`their-noise:…`, `remove-my-echo`) about the target
//! `in-stream:<room>/<user>`, which tells them apart.

use yapfast_core::application::MenuChoice;
use yapfast_core::volume::Cleaning;
use yapfast_lang::tr;

use anomi_vm::icon::Icon;
use anomi_vm::menu::Row;
use anomi_vm::voice::participant_menu::Intent as Card;

use super::{Make, choice_id, subs};

/// A watched stream's menu: this, the call's room id, `/`, and the
/// streamer's id, which has no `/`.
pub(crate) const IN_STREAM: &str = "in-stream:";

/// The call and the streamer a stream's menu is about.
pub(crate) fn stream_target(target: &str) -> Option<(&str, &str)> {
    target.strip_prefix(IN_STREAM)?.rsplit_once('/')
}

/// The menu's target for `user`'s stream in the call of `room`.
pub(crate) fn target(room: &str, user: &str) -> String {
    format!("{IN_STREAM}{room}/{user}")
}

/// The rows of the menu about `target`, the stream's sound cleaned as
/// `cleaning` says.
pub(crate) fn rows(target: &str, cleaning: Cleaning) -> Vec<Row> {
    let make = Make {
        target,
        person: true,
    };
    let noise = Row::opens(
        tr!("Suppress noise"),
        subs::their_noise(&make, cleaning.noise),
    )
    .subtitle(tr!(
        "Made for speech: music and game sound come out damaged"
    ))
    .wraps()
    .icon(Icon::NoiseBars);
    let echo = make
        .row(tr!("Remove my echo"), choice_id(MenuChoice::RemoveMyEcho))
        .icon(Icon::Volume)
        .checkbox(cleaning.own_echo);
    vec![noise, echo]
}

/// The card's own rows for `user`'s stream, as Discord's menu of a stream
/// starts: "Stop Watching" with the struck screen, then under a line the box
/// "Mute", ticked while the stream is `muted` here.
pub(crate) fn card_rows(user: &str, muted: bool) -> Vec<Row<Card>> {
    let stop = Row::new(tr!("Stop Watching"), Card::StopWatching).icon(Icon::ScreenShareOff);
    let mute = Row::new(tr!("Menu" => "Mute"), Card::MuteStream(user.to_owned()))
        .checkbox(muted)
        .plain()
        .separated();
    vec![stop, mute]
}

#[cfg(test)]
mod tests {
    use yapfast_core::noise::Suppressor;

    use super::*;

    #[test]
    fn a_streams_rows_choose_how_its_sound_is_cleaned_and_say_what_it_costs_music() {
        let target = target("discord:c3", "discord:u2");
        assert_eq!(stream_target(&target), Some(("discord:c3", "discord:u2")));
        assert_eq!(stream_target("in-voice:discord:c3/discord:u2"), None);
        let cleaning = Cleaning {
            noise: Suppressor::RNNoise,
            own_echo: true,
            ..Cleaning::NOTHING
        };
        let rows = rows(&target, cleaning);
        assert_eq!(rows.len(), 2);
        let noise = &rows[0];
        assert_eq!(noise.text, "Suppress noise");
        assert!(noise.subtitle.contains("speech"), "{}", noise.subtitle);
        let marked: Vec<bool> = noise.submenu.iter().map(|row| row.checked).collect();
        assert_eq!(marked, [false, true, false], "RNNoise chosen");
        assert!(rows[1].checkbox && rows[1].checked, "our echo taken out");
        let ids = |row: &Row| match &row.intent {
            Some(anomi_vm::Intent::Menu(anomi_vm::menu::Intent::Choose(choice))) => {
                (choice.target.clone(), choice.id.clone())
            }
            _ => panic!("a choice"),
        };
        assert_eq!(ids(&rows[1]), (target.clone(), "remove-my-echo".into()));
        assert_eq!(ids(&noise.submenu[2]).1, "their-noise:deepfilternet");
    }
}
