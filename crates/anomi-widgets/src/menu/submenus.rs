//! Submenus as the pointer leaves them (`super::hold`), in the simulator:
//! the window's context menu, and a card's row with its submenu beside it
//! (`Beside`), as the device menus lay theirs out. Each closes a moment
//! after the pointer went to another row or out of the menu, stays while
//! the pointer crosses a corner of another row on its way to it, and stays,
//! the keys having opened it, until a key or a press.

use std::time::{Duration, Instant};

use iced::keyboard::key::Named;
use iced::widget::column;
use iced::{Event, Point, Rectangle, Settings, Size, mouse, window};
use iced_test::Simulator;

use anomi_theme::{Element, Motion, Renderer, Theme};
use anomi_vm::menu::{Row, Spot};

use super::{beside, context_menu, item};

#[derive(Clone, Debug, PartialEq)]
enum Asked {
    Chose(&'static str),
    Hover(&'static str),
    Left,
}

type Ui<'a> = Simulator<'a, Asked, Theme, Renderer>;

const WINDOW: Size = Size::new(800.0, 600.0);
/// Off the menu, its submenu and the card.
const AWAY: Point = Point::new(700.0, 550.0);
/// Well within the wait, and well past it.
const SOON: Duration = Duration::from_millis(50);
const LATER: Duration = Duration::from_millis(400);

fn move_to(ui: &mut Ui<'_>, at: Point) {
    ui.point_at(at);
    ui.simulate([Event::Mouse(mouse::Event::CursorMoved { position: at })]);
}

/// A frame `wait` from now.
fn after(ui: &mut Ui<'_>, wait: Duration) {
    let at = Instant::now() + wait;
    ui.simulate([Event::Window(window::Event::RedrawRequested(at))]);
}

fn bounds(ui: &mut Ui<'_>, text: &str) -> Rectangle {
    ui.find(text)
        .unwrap_or_else(|_| panic!("{text}"))
        .visible_bounds()
        .expect("shown")
}

fn shows(ui: &mut Ui<'_>, text: &str) -> bool {
    ui.find(text).is_ok()
}

/// A person's menu with a submenu in the middle, as a call's has.
fn rows() -> Vec<Row<Asked>> {
    vec![
        Row::new("Profile", Asked::Chose("profile")),
        Row::opens(
            "Suppress noise",
            vec![
                Row::new("Off", Asked::Chose("off")).radio(true),
                Row::new("RNNoise", Asked::Chose("rnnoise")).radio(false),
                Row::new("DeepFilterNet", Asked::Chose("deepfilternet")).radio(false),
            ],
        ),
        Row::new("Remove my echo", Asked::Chose("echo")).checkbox(false),
        Row::new("Block", Asked::Chose("block")),
    ]
}

fn menu(rows: &[Row<Asked>]) -> Ui<'_> {
    let at = Spot::pointer(Point::new(100.0, 100.0));
    let menu = context_menu(rows, at, Motion::default());
    Simulator::with_size(Settings::default(), WINDOW, menu)
}

/// The menu with "Suppress noise"'s submenu opened by the pointer.
fn opened(rows: &[Row<Asked>]) -> Ui<'_> {
    let mut ui = menu(rows);
    let opener = bounds(&mut ui, "Suppress noise").center();
    move_to(&mut ui, opener);
    assert!(shows(&mut ui, "RNNoise"), "it opens on hover");
    ui
}

#[test]
fn a_submenu_closes_a_moment_after_the_pointer_went_to_another_row() {
    let rows = rows();
    let mut ui = opened(&rows);
    let echo = bounds(&mut ui, "Remove my echo").center();
    move_to(&mut ui, echo);
    after(&mut ui, SOON);
    assert!(shows(&mut ui, "RNNoise"), "within the wait");
    after(&mut ui, LATER);
    assert!(!shows(&mut ui, "RNNoise"), "gone once it ran out");
    // The row under the pointer took it.
    ui.tap_key(Named::Enter);
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Asked::Chose("echo")]
    );
}

#[test]
fn a_submenu_closes_a_moment_after_the_pointer_left_the_menu() {
    let rows = rows();
    for away in [AWAY, Point::new(900.0, 700.0)] {
        let mut ui = opened(&rows);
        move_to(&mut ui, away);
        after(&mut ui, SOON);
        assert!(shows(&mut ui, "RNNoise"), "within the wait: {away:?}");
        after(&mut ui, LATER);
        assert!(!shows(&mut ui, "RNNoise"), "gone: {away:?}");
    }
    // Back on it before the wait ran out, it stays.
    let mut ui = opened(&rows);
    move_to(&mut ui, AWAY);
    let off = bounds(&mut ui, "Off").center();
    move_to(&mut ui, off);
    after(&mut ui, LATER);
    assert!(shows(&mut ui, "RNNoise"));
}

#[test]
fn a_diagonal_move_across_another_row_reaches_the_submenu() {
    let rows = rows();
    let mut ui = opened(&rows);
    // Down and right over the next row's end, then the submenu's last row.
    let echo = bounds(&mut ui, "Remove my echo");
    let mut corner = echo.center();
    corner.x = echo.x + echo.width - 4.0;
    move_to(&mut ui, corner);
    after(&mut ui, SOON);
    let deep = bounds(&mut ui, "DeepFilterNet").center();
    move_to(&mut ui, deep);
    after(&mut ui, LATER);
    assert!(shows(&mut ui, "RNNoise"), "still open");
    // The row reached is lit: Enter chooses it, not the one crossed.
    ui.tap_key(Named::Enter);
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Asked::Chose("deepfilternet")]
    );
}

#[test]
fn a_submenu_the_keys_opened_stays_until_a_key_or_a_press() {
    let rows = rows();
    let mut ui = menu(&rows);
    move_to(&mut ui, AWAY);
    for key in [Named::ArrowDown, Named::ArrowDown, Named::ArrowRight] {
        ui.tap_key(key);
    }
    assert!(shows(&mut ui, "RNNoise"), "Right opened it");
    move_to(&mut ui, Point::new(650.0, 500.0));
    after(&mut ui, LATER);
    assert!(shows(&mut ui, "RNNoise"), "the pointer away");
    let block = bounds(&mut ui, "Block").center();
    move_to(&mut ui, block);
    after(&mut ui, LATER);
    assert!(shows(&mut ui, "RNNoise"), "the pointer on another row");
    ui.tap_key(Named::ArrowLeft);
    assert!(!shows(&mut ui, "RNNoise"), "Left closed it");
    ui.tap_key(Named::ArrowRight);
    assert!(shows(&mut ui, "RNNoise"));
    ui.simulate([Event::Mouse(mouse::Event::ButtonPressed(
        mouse::Button::Left,
    ))]);
    assert!(!shows(&mut ui, "RNNoise"), "a press closed it");
}

/// A card's rows as a device menu has them: the device, its submenu beside
/// the card, and Voice Settings.
fn card_rows() -> Vec<Row<Asked>> {
    vec![
        Row::new("Input Device", Asked::Hover("device")).submenu(vec![
            Row::new("Default", Asked::Chose("default")).radio(true),
            Row::new("Headset", Asked::Chose("headset")).radio(false),
            Row::new("Webcam", Asked::Chose("webcam")).radio(false),
        ]),
        Row::new("Voice Settings", Asked::Chose("settings")),
    ]
}

/// The card, its first row's submenu showing as the view-model has it.
fn card(rows: &[Row<Asked>]) -> Ui<'_> {
    use iced::Widget as _;
    let motion = Motion::default();
    let device = item(&rows[0], motion)
        .arrow(true)
        .on_hover(Asked::Hover("device"));
    let settings = item(&rows[1], motion).on_hover(Asked::Hover("settings"));
    let view: Element<'_, Asked> = column![
        beside(device, &rows[0].submenu, true, Asked::Left, motion).reach(6.0),
        settings,
    ]
    .width(255)
    .padding(6)
    .boxed();
    Simulator::with_size(Settings::default(), WINDOW, view)
}

/// The card with the pointer on the device's row: the submenu is its.
fn pointed(rows: &[Row<Asked>]) -> Ui<'_> {
    let mut ui = card(rows);
    let device = bounds(&mut ui, "Input Device").center();
    move_to(&mut ui, device);
    ui
}

#[test]
fn a_cards_submenu_asks_to_close_a_moment_after_the_pointer_left() {
    let rows = card_rows();
    let mut ui = pointed(&rows);
    move_to(&mut ui, AWAY);
    after(&mut ui, SOON);
    assert_eq!(ui.drain().collect::<Vec<_>>(), [Asked::Hover("device")]);
    after(&mut ui, LATER);
    assert_eq!(ui.into_messages().collect::<Vec<_>>(), [Asked::Left]);
}

#[test]
fn a_cards_submenu_waits_before_another_row_takes_the_pointer() {
    let rows = card_rows();
    let mut ui = pointed(&rows);
    let settings = bounds(&mut ui, "Voice Settings").center();
    move_to(&mut ui, settings);
    after(&mut ui, SOON);
    assert_eq!(ui.drain().collect::<Vec<_>>(), [Asked::Hover("device")]);
    after(&mut ui, LATER);
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Asked::Left, Asked::Hover("settings")]
    );
}

#[test]
fn a_diagonal_move_across_the_next_row_reaches_a_cards_submenu() {
    let rows = card_rows();
    let mut ui = pointed(&rows);
    let settings = bounds(&mut ui, "Voice Settings");
    let mut corner = settings.center();
    corner.x = settings.x + settings.width - 4.0;
    move_to(&mut ui, corner);
    after(&mut ui, SOON);
    let webcam = bounds(&mut ui, "Webcam").center();
    move_to(&mut ui, webcam);
    after(&mut ui, LATER);
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Asked::Hover("device")],
        "neither closed nor taken by the row crossed"
    );
}

#[test]
fn a_cards_submenu_the_keys_opened_stays_until_a_press() {
    let rows = card_rows();
    // Shown from the start, the pointer never on its row: the keys'.
    let mut ui = card(&rows);
    move_to(&mut ui, AWAY);
    after(&mut ui, LATER);
    let settings = bounds(&mut ui, "Voice Settings").center();
    move_to(&mut ui, settings);
    after(&mut ui, LATER);
    assert_eq!(
        ui.drain().collect::<Vec<_>>(),
        [],
        "it stays, the row waits"
    );
    // A press asks it to close, and goes on to the row under the pointer.
    ui.simulate(iced_test::simulator::click());
    assert_eq!(
        ui.into_messages().collect::<Vec<_>>(),
        [Asked::Left, Asked::Chose("settings")]
    );
}
