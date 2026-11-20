//! The stream's two menus that are the window's context menu (`vm.menu`),
//! their rows carrying the window's intent:
//!
//! - [`gear`], the Go Live dialog's gear (`discord-golive-2-gear-menu.png`):
//!   "Stream Mode" with Discord's three modes as radio rows, the presets
//!   with what they set under their names; "Screen Resolution" and "Frame
//!   Rate" with their choices beside; "Mute Stream Audio"; "Advanced",
//!   whose submenu has "Hide Stream Preview" (`discord-golive-3-gear-
//!   advanced.png`) and what this client has beyond Discord: the bitrate
//!   limit, the encoder's higher quality and where streams encode.
//! - [`own_menu`], a right click on one's own stream's tile
//!   (`discord-own-stream-menu.png`): "Stop Streaming" first, in red;
//!   "Stream Quality" with the resolutions and frame rates beside it;
//!   "Share Stream Audio"; and under a line "More Settings", with "Change
//!   Windows" and the rows of the gear's "Advanced". Discord's "Pop Out
//!   Stream" has no place yet: this client has no window of its own for a
//!   stream.
//!
//! A box to tick leaves the menu open, and the app makes its rows again;
//! any other choice closes it.

use yapfast_lang::tr;

use anomi_vm::icon::Icon;
use anomi_vm::menu::Row;
use anomi_vm::voice::share::{FRAME_RATES, Intent, Mode, Resolution, Share, Sound};

use super::rows::{limits, mode_name, resolution};

/// The window's intent for one of the stream's.
fn to(intent: Intent) -> anomi_vm::Intent {
    anomi_vm::Intent::from(intent)
}

/// The gear's menu, from the dialog's picks.
pub fn gear(share: &Share) -> Vec<Row> {
    let mode = share.mode();
    let preset = |chosen: Mode, subtitle: &str| {
        let row = Row::new(mode_name(chosen), to(Intent::Mode(chosen)));
        let row = if subtitle.is_empty() {
            row
        } else {
            row.subtitle(subtitle)
        };
        row.radio(mode == chosen).plain()
    };
    let muted = share.sound == Sound::None;
    vec![
        Row::heading(tr!("Stream Mode")),
        preset(Mode::Games, tr!("Smoother video (1440p, 60 fps)")),
        preset(Mode::Screen, tr!("Sharper text (Source, 15 fps)")),
        preset(Mode::Custom, ""),
        Row::opens(tr!("Screen Resolution"), resolutions(share))
            .separated()
            .plain(),
        Row::opens(tr!("Frame Rate"), rates(share)).plain(),
        Row::new(tr!("Mute Stream Audio"), to(Intent::Sound(heard(!muted))))
            .checkbox(muted)
            .separated()
            .plain(),
        Row::opens(tr!("Advanced Options"), advanced(share)).plain(),
    ]
}

/// The menu of one's own stream's tile, from the stream's picks; "Change
/// Windows" waits while the stream starts (`changeable`).
pub fn own_menu(share: &Share, changeable: bool) -> Vec<Row> {
    let sharing = share.sound != Sound::None;
    let mut quality = vec![Row::heading(tr!("RESOLUTION"))];
    quality.extend(resolutions(share));
    quality.push(Row::heading(tr!("FRAME RATE")).separated());
    quality.extend(rates(share));
    let mut more = vec![
        Row::new(tr!("Change Windows"), to(Intent::Change))
            .disabled(!changeable)
            .plain(),
    ];
    let mut advanced = advanced(share);
    if let Some(first) = advanced.first_mut() {
        first.separated = true;
    }
    more.extend(advanced);
    vec![
        Row::new(tr!("Stop streaming"), to(Intent::Stop))
            .icon(Icon::ScreenShareOff)
            .danger(),
        Row::opens(tr!("Stream quality"), quality).plain(),
        Row::new(
            tr!("Share Stream Audio"),
            to(Intent::Sound(heard(!sharing))),
        )
        .checkbox(sharing)
        .plain(),
        Row::opens(tr!("More Settings"), more).separated().plain(),
    ]
}

/// The sound a box ticked or unticked asks for: every application's, or
/// none.
fn heard(on: bool) -> Sound {
    if on { Sound::All } else { Sound::None }
}

/// The resolutions, as radio rows.
fn resolutions(share: &Share) -> Vec<Row> {
    (Resolution::ALL.into_iter())
        .map(|choice| {
            Row::new(resolution(choice), to(Intent::Resolution(choice)))
                .radio(share.resolution == choice)
                .plain()
        })
        .collect()
}

/// The frame rates, as radio rows.
fn rates(share: &Share) -> Vec<Row> {
    (FRAME_RATES.into_iter())
        .map(|fps| {
            Row::new(tr!("{} fps", fps), to(Intent::Fps(fps)))
                .radio(share.fps == fps)
                .plain()
        })
        .collect()
}

/// "Hide Stream Preview", then the bitrate limit, the encoder's higher
/// quality and where streams encode, each group under its title.
fn advanced(share: &Share) -> Vec<Row> {
    let mut rows = vec![
        Row::new(
            tr!("Hide Stream Preview"),
            to(Intent::HidePreview(!share.preview_hidden)),
        )
        .checkbox(share.preview_hidden)
        .plain(),
        Row::heading(tr!("BITRATE LIMIT")).separated(),
    ];
    rows.extend(limits(share, to));
    rows.push(Row::heading(tr!("ENCODING")).separated());
    rows.push(
        Row::new(
            tr!("Higher quality"),
            to(Intent::HighQuality(!share.high_quality)),
        )
        .subtitle(tr!("Sharper, at twice the CPU"))
        .checkbox(share.high_quality)
        .plain(),
    );
    if !share.encoders.is_empty() {
        rows.push(Row::heading(tr!("ENCODE STREAMS ON")).separated());
        rows.extend((share.encoders.iter().enumerate()).map(|(at, name)| {
            Row::new(name.as_str(), to(Intent::Encoder(at)))
                .radio(share.encoder == at)
                .plain()
        }));
    }
    rows
}
