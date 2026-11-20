//! What the dialog and the stream's menu say, made from the picks with the
//! text translated: the windows to pick, the mode and quality along the
//! dialog's bottom, and while live the stream's menu's rows, its submenus
//! with radio marks under the groups' titles, as Discord's client has them.
//! A row's view borrows its row, so [`built`] stores them in the view-model
//! (`Share::rows`, `Share::live`); the projection calls it after setting
//! the rest, and so do the shots and tests.

use yapfast_lang::tr;

use anomi_vm::menu::Row;
use anomi_vm::voice::share::{
    BITRATES, FRAME_RATES, Intent, Live, Mode, Part, Resolution, Rows, Share, Sound,
};

/// What the menus call a resolution.
pub fn resolution(resolution: Resolution) -> &'static str {
    match resolution {
        Resolution::P720 => "720p",
        Resolution::P1080 => "1080p",
        Resolution::P1440 => "1440p",
        Resolution::Source => tr!("Source"),
    }
}

/// What the menus call a bitrate limit, in kbit/s; Auto for none. The
/// steps are whole texts, as each language writes its decimals.
pub fn bitrate(kbps: Option<u32>) -> String {
    match kbps {
        None => tr!("Auto").to_owned(),
        Some(2_000) => tr!("2 Mbit/s").to_owned(),
        Some(3_500) => tr!("3.5 Mbit/s").to_owned(),
        Some(5_000) => tr!("5 Mbit/s").to_owned(),
        Some(6_000) => tr!("6 Mbit/s").to_owned(),
        Some(8_000) => tr!("8 Mbit/s").to_owned(),
        Some(9_000) => tr!("9 Mbit/s").to_owned(),
        Some(other) => tr!("{} kbit/s", other),
    }
}

/// The limits, Auto first with what it sends, as radio rows.
pub(super) fn limits<I>(share: &Share, intent: impl Fn(Intent) -> I) -> Vec<Row<I>> {
    std::iter::once(None)
        .chain(BITRATES.into_iter().map(Some))
        .map(|kbps| {
            let row = Row::new(bitrate(kbps), intent(Intent::Bitrate(kbps)));
            let row = if kbps.is_none() {
                row.subtitle(tr!("3.5 Mbit/s up to 720p, 9 above"))
            } else {
                row
            };
            row.radio(share.max_kbps == kbps).plain()
        })
        .collect()
}

/// A stream mode's name.
pub(super) fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Games => tr!("Games"),
        Mode::Screen => tr!("Screen Share"),
        Mode::Custom => tr!("Custom Quality"),
    }
}

/// `share` with its rows made from its picks.
pub fn built(mut share: Share) -> Share {
    let windows = (share.windows.iter().enumerate())
        .map(|(at, title)| {
            let index = share.screens.len() + at;
            Row::new(title.as_str(), Intent::Source(index)).checked(share.source == Some(index))
        })
        .collect();
    let sound = match share.sound {
        Sound::All => tr!("System sound"),
        Sound::None => tr!("No sound"),
    };
    share.rows = Rows {
        windows,
        mode: mode_name(share.mode()).to_owned(),
        quality: format!(
            "{} · {} · {}",
            resolution(share.resolution),
            tr!("{}fps", share.fps),
            sound
        ),
    };
    if let Some(mut live) = share.live.take() {
        stream(&share, &mut live);
        share.live = Some(live);
    }
    share
}

/// What the stream goes out at, as the stream's menu and the dock say it:
/// "1080p · 60 fps".
pub fn quality(resolution: Resolution, fps: u32) -> String {
    tr!("{} · {} fps", self::resolution(resolution), fps)
}

/// The stream's menu's rows, from the picks: "Stream Quality" with the
/// frame rates, the resolutions, the encoding and the bitrate limit beside
/// it, "Sound" with its choices, "Change Windows" and "Stop Streaming".
fn stream(share: &Share, live: &mut Live) {
    let mut rates = vec![Row::heading(tr!("FRAME RATE"))];
    rates.extend(FRAME_RATES.into_iter().map(|fps| {
        Row::new(tr!("{} fps", fps), Intent::Fps(fps))
            .radio(share.fps == fps)
            .plain()
    }));
    rates.push(Row::heading(tr!("RESOLUTION")).separated());
    rates.extend(Resolution::ALL.into_iter().map(|choice| {
        Row::new(resolution(choice), Intent::Resolution(choice))
            .radio(share.resolution == choice)
            .plain()
    }));
    rates.push(Row::heading(tr!("ENCODING")).separated());
    rates.push(
        Row::new(
            tr!("Higher quality"),
            Intent::HighQuality(!share.high_quality),
        )
        .subtitle(tr!("Sharper, at twice the CPU"))
        .checkbox(share.high_quality)
        .plain(),
    );
    rates.push(Row::heading(tr!("BITRATE LIMIT")).separated());
    rates.extend(limits(share, |intent| intent));
    let choice = |text: &str, sound: Sound| {
        Row::new(text, Intent::Sound(sound))
            .radio(share.sound == sound)
            .plain()
    };
    let sounds = vec![
        choice(tr!("All applications"), Sound::All).subtitle(tr!("Except Yapfast's own sound")),
        choice(tr!("No sound"), Sound::None),
    ];
    let heard = match share.sound {
        Sound::All => tr!("All applications"),
        Sound::None => tr!("No sound"),
    };
    let opens = |part| Intent::Hover(Some(part));
    live.quality = Row::new(tr!("Stream Quality"), opens(Part::Quality))
        .subtitle(quality(share.resolution, share.fps))
        .submenu(rates)
        .plain();
    live.sound = Row::new(tr!("ShareMenu" => "Sound"), opens(Part::Sound))
        .subtitle(heard)
        .submenu(sounds)
        .plain();
    live.change = Row::new(tr!("Change Windows"), Intent::Change)
        .disabled(!live.changeable)
        .plain();
    live.stop = Row::new(tr!("Stop Streaming"), Intent::Stop)
        .danger()
        .plain();
    live.settle();
}
