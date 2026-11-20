//! Samples of a participant's menu for tests and shots, after the fake
//! Discord's data: task V11's. The card is `vm.voice.participant_menu`; the
//! rows under it are the window's context menu for the target
//! `in-voice:<room>/<user>` (`src/ui/menus.rs`), as `menu/voice_person.png`
//! has them, each sending the choice its id names. A watched stream's tile
//! has the same card, with rows of its own over its volume, over its own
//! rows, for `in-stream:<room>/<user>` ([`stream`], [`stream_muted`]).

use iced::{Point, Size};
use yapfast_lang::tr;

use crate::Vm;
use crate::fixtures::shell;
use crate::icon::Icon;
use crate::menu::{Menu, Row, Spot};
use crate::shell::Layer;
use crate::voice::participant_menu::{Intent as CardIntent, ParticipantMenu};

use super::stage;

/// Where `voice/participant_menu.png` right-clicked Carol's tile, and the
/// rows' size and the window's there: the card lands at (670, 8).
pub const POINTER: Point = Point::new(670.0, 550.0);
const ROWS: Size = Size::new(WIDTH, 644.0);
const WINDOW: Size = Size::new(1400.0, 860.0);

/// The width the rows give the card in the golden window.
pub const WIDTH: f32 = 274.0;

/// Lounge's room id, and Carol's and Bob's user ids, as the fake has them.
const ROOM: &str = "discord:c3";
pub const CAROL: &str = "discord:u3";
pub const BOB: &str = "discord:u2";

fn target() -> String {
    format!("in-voice:{ROOM}/{CAROL}")
}

fn row(text: &str, id: &str) -> Row {
    Row::new(text, crate::menu::Choice::person(target(), id).into())
}

/// What a call's menu offers someone this account is friends with, in
/// the order `voice/participant_menu.png` shows (`menu` and `label` in
/// `src/ui/menus.rs`), with the moderator's rows at the end.
pub fn rows(server_muted: bool) -> Vec<Row> {
    vec![
        row(tr!("Profile"), "profile").icon(Icon::User),
        row(tr!("Mention"), "mention").icon(Icon::AtSign),
        row(yapfast_lang::text_in("Menu", "Message"), "message").icon(Icon::MessageCircle),
        row(tr!("Start call"), "call").icon(Icon::Phone),
        row(tr!("Add note"), "add-note")
            .subtitle(tr!("Only visible to you"))
            .icon(Icon::StickyNote),
        row(tr!("Add friend nickname"), "add-nickname").icon(Icon::UserPen),
        row(tr!("Menu" => "Mute"), "local-mute")
            .icon(Icon::MicOff)
            .checkbox(false)
            .separated(),
        row(tr!("Mute Soundboard"), "soundboard-mute")
            .icon(Icon::Soundboard)
            .checkbox(false),
        Row::opens(
            tr!("Invite to server"),
            vec![row("Server", "invite-to:discord:g1")],
        )
        .icon(Icon::UserPlus)
        .separated(),
        row(tr!("Remove friend"), "unfriend")
            .icon(Icon::UserMinus)
            .danger(),
        row(tr!("Ignore"), "ignore").icon(Icon::EyeOff),
        row(tr!("Block"), "block").icon(Icon::Ban).danger(),
        Row::opens(
            tr!("Roles"),
            vec![row("Moderator", "role:r1").checkbox(false)],
        )
        .icon(Icon::Shield)
        .separated(),
        Row::opens(tr!("Move to"), vec![row("Lounge 2", "move-to:discord:c4")]).icon(Icon::Forward),
        row(tr!("Server mute"), "server-mute")
            .icon(Icon::MicOff)
            .checkbox(server_muted)
            .danger()
            .separated(),
        row(tr!("Server deafen"), "server-deafen")
            .icon(Icon::HeadphoneOff)
            .checkbox(false)
            .danger(),
        row(tr!("Menu" => "Disconnect"), "disconnect")
            .icon(Icon::PhoneOff)
            .danger(),
    ]
}

/// The card for Carol, at her usual volumes.
pub fn card() -> ParticipantMenu {
    ParticipantMenu {
        person: CAROL.into(),
        name: "Carol".into(),
        joined: true,
        width: WIDTH,
        ..ParticipantMenu::default()
    }
}

/// The window with the card open at the pointer and the rows right under.
fn open(card: ParticipantMenu, rows: Vec<Row>) -> Vm {
    open_over(card, rows, ROWS)
}

/// [`open`], the rows `size` tall.
fn open_over(card: ParticipantMenu, rows: Vec<Row>, size: Size) -> Vm {
    let mut vm = stage::window(stage::lounge());
    let (top, below) = card.place(POINTER, size, WINDOW);
    vm.menu = Menu {
        narrowest: Some(crate::voice::participant_menu::WIDTH),
        headed: true,
        ..Menu::new(1, Spot::pointer(below), rows)
    };
    vm.voice.participant_menu = card;
    shell::open_at(vm, Layer::ParticipantMenu, Spot::pointer(top))
}

/// `voice/participant_menu.png`: Carol's menu.
pub fn carol() -> Vm {
    open(card(), rows(false))
}

/// Carol's menu in a Discord call with the rows that clean her voice, under
/// the mutes: RNNoise takes out 80% of her noise and this account's echo
/// comes out of her voice 20 dB, each with its slider on the card.
pub fn cleaning() -> Vm {
    let card = ParticipantMenu {
        noise_strength: Some(80),
        echo_depth: Some(20),
        ..card()
    };
    let mut rows = rows(false);
    let after = rows
        .iter()
        .position(|row| row.text == tr!("Mute Soundboard"))
        .map_or(rows.len(), |at| at + 1);
    let below = rows.split_off(after);
    let suppressors = vec![
        row(tr!("Off"), "their-noise:off").radio(false).plain(),
        row("RNNoise", "their-noise:rnnoise")
            .subtitle(tr!("Light on the processor"))
            .radio(true)
            .plain(),
        row("DeepFilterNet", "their-noise:deepfilternet")
            .subtitle(tr!("Stronger, but uses more of the processor"))
            .wraps()
            .radio(false)
            .plain(),
    ];
    rows.push(
        Row::opens(tr!("Suppress their noise"), suppressors)
            .subtitle("RNNoise")
            .icon(Icon::NoiseBars),
    );
    rows.push(
        row(tr!("Remove my echo"), "remove-my-echo")
            .icon(Icon::Volume)
            .checkbox(true),
    );
    rows.extend(below);
    // A row of 44 px with its subtitle and one of 32 more than Carol's
    // menu, each with the 2 px between rows.
    open_over(card, rows, Size::new(WIDTH, ROWS.height + 80.0))
}

/// [`cleaning`] with "Suppress their noise" open: none, the light RNNoise,
/// chosen, and the strong DeepFilterNet, which says it costs more of the
/// processor.
pub fn their_noise() -> Vm {
    let mut vm = cleaning();
    vm.menu.submenu = (vm.menu.rows.iter()).position(|row| row.text == tr!("Suppress their noise"));
    vm
}

/// Someone sharing their screen: a second slider, for their stream.
pub fn sharing() -> Vm {
    open(
        ParticipantMenu {
            live: true,
            stream_volume: 60,
            ..card()
        },
        rows(false),
    )
}

/// Only the card, as it shows without a menu under it (a call that offers
/// no rows to this account).
pub fn alone() -> Vm {
    let mut vm = stage::window(stage::lounge());
    let card = ParticipantMenu {
        joined: false,
        width: crate::voice::participant_menu::WIDTH,
        ..card()
    };
    let (top, _) = card.place(POINTER, Size::ZERO, WINDOW);
    vm.voice.participant_menu = card;
    shell::open_at(vm, Layer::ParticipantMenu, Spot::pointer(top))
}

/// The menu of Bob's stream's tile while Alice watches it, right-clicked on
/// the large stream, as Discord's starts: "Stop Watching", a line, "Mute"
/// and the stream's volume on the card, then how much of its noise RNNoise
/// takes out and how far Alice's echo comes out of it, both sliders shown;
/// the rows under it choose them, "Suppress noise" saying what it does to
/// music.
pub fn stream() -> Vm {
    watched_stream(true, false)
}

/// [`stream`] with nothing cleaning its sound, and the stream muted here:
/// "Mute" ticked, its volume kept as it was.
pub fn stream_muted() -> Vm {
    watched_stream(false, true)
}

/// A stream's own rows on its card, as the app makes them
/// (`project::menus::stream::card_rows`): "Stop Watching" with the struck
/// screen, then under a line the box "Mute", ticked while `muted`.
pub fn stream_rows(person: &str, muted: bool) -> Vec<Row<CardIntent>> {
    vec![
        Row::new(tr!("Stop Watching"), CardIntent::StopWatching).icon(Icon::ScreenShareOff),
        Row::new(tr!("Menu" => "Mute"), CardIntent::MuteStream(person.into()))
            .checkbox(muted)
            .plain()
            .separated(),
    ]
}

/// Bob's stream's menu, its sound `cleaned` (RNNoise, Alice's echo out) or
/// not, and `muted` here or not.
fn watched_stream(cleaned: bool, muted: bool) -> Vm {
    let target = format!("in-stream:{ROOM}/{BOB}");
    let row = |text: &str, id: &str| {
        Row::new(text, crate::menu::Choice::person(target.clone(), id).into())
    };
    let suppressors = vec![
        row(tr!("Off"), "their-noise:off").radio(!cleaned).plain(),
        row("RNNoise", "their-noise:rnnoise")
            .subtitle(tr!("Light on the processor"))
            .radio(cleaned)
            .plain(),
        row("DeepFilterNet", "their-noise:deepfilternet")
            .subtitle(tr!("Stronger, but uses more of the processor"))
            .wraps()
            .radio(false)
            .plain(),
    ];
    let rows = vec![
        Row::opens(tr!("Suppress noise"), suppressors)
            .subtitle(tr!(
                "Made for speech: music and game sound come out damaged"
            ))
            .wraps()
            .icon(Icon::NoiseBars),
        row(tr!("Remove my echo"), "remove-my-echo")
            .icon(Icon::Volume)
            .checkbox(cleaned),
    ];
    let card = ParticipantMenu {
        person: BOB.into(),
        name: tr!("{}'s screen", "Bob"),
        live: true,
        stream: true,
        stream_volume: 54,
        noise_strength: cleaned.then_some(100),
        echo_depth: cleaned.then_some(20),
        joined: true,
        width: WIDTH,
        rows: stream_rows(BOB, muted),
        ..ParticipantMenu::default()
    };
    let mut vm = stage::window(stage::watching());
    let (top, below) = card.place(STREAM_AT, Size::new(WIDTH, 110.0), WINDOW);
    vm.menu = Menu {
        narrowest: Some(crate::voice::participant_menu::WIDTH),
        headed: true,
        ..Menu::new(1, Spot::pointer(below), rows)
    };
    vm.voice.participant_menu = card;
    shell::open_at(vm, Layer::ParticipantMenu, Spot::pointer(top))
}

/// Where [`stream`] right-clicked Bob's stream, large on the stage.
const STREAM_AT: Point = Point::new(900.0, 300.0);
