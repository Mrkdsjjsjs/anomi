//! The Go Live dialog and the stream's menus: driving
//! `anomi_voice::share::view` over its fixtures in the simulator
//! (`super::support`), and the gear's and one's own stream's menus as rows.

use iced::Size;
use iced::keyboard::key::Named;
use iced_test::{Simulator, selector};

use anomi_theme::{Renderer, Theme};
use anomi_vm::fixtures::voice::share as samples;
use anomi_vm::menu::{Key, Row};
use anomi_vm::voice::share::{Intent, Offer, Resolution, Share, Sound, Tab};
use anomi_voice::share::{self, built, gear, own_menu};

use super::support::{asked, front, settings};

/// Room for the dialog, as Discord's.
const DIALOG: Size = Size::new(1000.0, 640.0);
/// Room for the stream's menu and a submenu beside it.
const LIVE: Size = Size::new(900.0, 700.0);

fn simulate(vm: &Share) -> Simulator<'_, Intent, Theme, Renderer> {
    Simulator::with_size(settings(), DIALOG, share::view(vm, front()))
}

/// Clicks `text` and returns what the dialog asked for.
fn click(vm: &Share, text: &str) -> Vec<Intent> {
    let mut ui = simulate(vm);
    ui.click(text).unwrap();
    asked(ui)
}

/// Presses the button at the bottom right.
fn go(vm: &Share) -> Vec<Intent> {
    let mut ui = simulate(vm);
    let _ = ui.click(selector::id(share::GO));
    asked(ui)
}

/// A window menu's row by its text, among its submenus too.
fn row<'a>(rows: &'a [Row], text: &str) -> &'a Row {
    rows.iter()
        .find_map(|row| {
            (row.text == text)
                .then_some(row)
                .or_else(|| row.submenu.iter().find(|row| row.text == text))
        })
        .unwrap_or_else(|| panic!("no row {text}"))
}

#[test]
fn where_the_system_picks_the_cards_ask_it_for_their_kind() {
    let vm = built(samples::dialog());
    let mut ui = simulate(&vm);
    for shown in [
        "Applications",
        "Devices",
        "Share your screen, a window or an application",
        "Choose What to Share",
    ] {
        assert!(ui.find(shown).is_ok(), "{shown}");
    }
    drop(ui);
    assert_eq!(
        click(&vm, "Share Entire Screen"),
        [Intent::Ask(Offer::Screen)]
    );
    assert_eq!(click(&vm, "Share a Window"), [Intent::Ask(Offer::Window)]);
    assert_eq!(go(&vm), [Intent::Ask(Offer::Either)]);
}

#[test]
fn the_mode_and_quality_show_along_the_bottom() {
    let vm = built(samples::dialog());
    let mut ui = simulate(&vm);
    assert!(ui.find("Custom Quality").is_ok());
    assert!(ui.find("720p · 60fps · System sound").is_ok());
    drop(ui);
    let muted = built(Share {
        sound: Sound::None,
        resolution: Resolution::Source,
        fps: 15,
        ..samples::dialog()
    });
    assert_eq!(muted.rows.mode, "Screen Share");
    assert_eq!(muted.rows.quality, "Source · 15fps · No sound");
}

#[test]
fn elsewhere_screens_and_windows_are_listed_and_picked() {
    let vm = built(samples::listed());
    assert!(simulate(&vm).find("Share Entire Screen").is_err());
    assert_eq!(click(&vm, "Screen 2"), [Intent::Source(1)]);
    assert_eq!(click(&vm, "Terminal"), [Intent::Source(2)]);
    assert_eq!(click(&vm, "Devices"), [Intent::Tab(Tab::Devices)]);
    assert!(matches!(
        go(&vm)[..],
        [Intent::GoLive {
            source: 0,
            resolution: Resolution::P1080,
            fps: 30,
            high_quality: false,
        }]
    ));
    let mut ui = simulate(&vm);
    assert!(ui.find("Start Streaming").is_ok());
}

#[test]
fn a_screen_alone_is_still_chosen_by_its_card() {
    let vm = built(samples::one_screen());
    assert_eq!(click(&vm, "Screen 1"), [Intent::Source(0)]);
}

#[test]
fn devices_lists_the_cameras_to_stream() {
    let vm = built(samples::devices());
    assert_eq!(vm.tab, Tab::Devices);
    assert_eq!(click(&vm, "OBS Virtual Camera"), [Intent::Source(1)]);
    assert!(matches!(go(&vm)[..], [Intent::GoLive { source: 0, .. }]));
    assert_eq!(click(&vm, "Applications"), [Intent::Tab(Tab::Apps)]);
}

#[test]
fn without_a_camera_devices_says_so_and_waits() {
    let vm = built(samples::no_devices());
    let mut ui = simulate(&vm);
    assert!(ui.find("No capture devices found.").is_ok());
    drop(ui);
    assert!(!vm.can_go());
    assert!(go(&vm).is_empty(), "disabled: it asks for nothing");
}

#[test]
fn switching_waits_for_a_pick_then_switches() {
    let vm = built(samples::switching());
    let mut ui = simulate(&vm);
    assert!(ui.find("Switch").is_ok());
    drop(ui);
    assert!(go(&vm).is_empty());
    let picked = built(Share {
        source: Some(1),
        ..samples::switching()
    });
    assert_eq!(go(&picked), [Intent::Switch(1)]);
}

#[test]
fn the_gears_menu_has_discords_modes_and_ours_under_advanced() {
    let vm = built(samples::tuned());
    let rows = gear(&vm);
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Stream Mode",
            "Games",
            "Screen Share",
            "Custom Quality",
            "Screen Resolution",
            "Frame Rate",
            "Mute Stream Audio",
            "Advanced Options",
        ]
    );
    assert!(rows[0].heading);
    assert!(row(&rows, "Custom Quality").checked, "720p at 60");
    assert_eq!(
        row(&rows, "Games").subtitle,
        "Smoother video (1440p, 60 fps)"
    );
    let sizes: Vec<&str> = (row(&rows, "Screen Resolution").submenu.iter())
        .map(|row| row.text.as_str())
        .collect();
    assert_eq!(sizes, ["720p", "1080p", "1440p", "Source"]);
    let games = built(Share {
        resolution: Resolution::P1440,
        fps: 60,
        ..samples::tuned()
    });
    assert_eq!(games.rows.mode, "Games");
    assert_eq!(games.rows.quality, "1440p · 60fps · System sound");
    assert!(!row(&rows, "Mute Stream Audio").checked);
    let advanced = &row(&rows, "Advanced Options").submenu;
    assert_eq!(advanced[0].text, "Hide Stream Preview");
    assert!(row(advanced, "6 Mbit/s").checked);
    assert!(row(advanced, "Higher quality").checked);
    assert!(!row(advanced, "Auto").checked, "the limit's Auto, first");
    let encoders: Vec<&Row> = (advanced.iter())
        .skip_while(|row| row.text != "ENCODE STREAMS ON")
        .skip(1)
        .collect();
    let chosen: Vec<(&str, bool)> = (encoders.iter())
        .map(|row| (row.text.as_str(), row.radio && row.checked))
        .collect();
    assert_eq!(
        chosen,
        [("Auto", true), ("Radeon 610M", false), ("Processor", false)]
    );
}

#[test]
fn the_own_streams_menu_is_discords() {
    let vm = built(Share {
        switching: true,
        ..samples::dialog()
    });
    let rows = own_menu(&vm, true);
    let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "Stop streaming",
            "Stream quality",
            "Share Stream Audio",
            "More Settings"
        ]
    );
    assert!(rows[0].danger && rows[0].icon.is_some());
    assert!(rows[2].checkbox && rows[2].checked);
    assert!(row(&rows, "60 fps").checked && row(&rows, "720p").checked);
    let more = &rows[3].submenu;
    assert_eq!(more[0].text, "Change Windows");
    assert!(!more[0].disabled);
    assert!(own_menu(&vm, false)[3].submenu[0].disabled);
}

/// The stream's menu in a window with room for its submenus.
fn live(vm: &Share) -> Simulator<'_, Intent, Theme, Renderer> {
    Simulator::with_size(settings(), LIVE, share::view(vm, front()))
}

#[test]
fn while_live_the_menu_is_the_streams_own() {
    let vm = built(samples::live());
    let mut ui = live(&vm);
    for shown in [
        "Stream Quality",
        "Sound",
        "Change Windows",
        "Stop Streaming",
    ] {
        assert!(ui.find(shown).is_ok(), "{shown}");
    }
    for hidden in ["FRAME RATE", "60 fps", "Applications", "Switch"] {
        assert!(ui.find(hidden).is_err(), "{hidden}");
    }
    let stream = vm.live.as_ref().unwrap();
    assert_eq!(stream.quality.subtitle, "720p · 60 fps");
    assert_eq!(stream.sound.subtitle, "All applications");
    assert!(stream.stop.danger);
}

#[test]
fn its_quality_has_discords_groups_beside_it_and_marks_the_stream() {
    let vm = built(samples::live_quality());
    let mut ui = live(&vm);
    let head = ui.find("Stream Quality").unwrap().bounds();
    let rates = ui.find("FRAME RATE").unwrap().bounds();
    assert!(rates.x > head.x + head.width, "beside the card: {rates:?}");
    assert!(ui.find("BITRATE LIMIT").is_ok());
    drop(ui);
    let rows = &vm.live.as_ref().unwrap().quality.submenu;
    let marked: Vec<&str> = (rows.iter())
        .filter(|row| row.radio && row.checked)
        .map(|row| row.text.as_str())
        .collect();
    assert_eq!(marked, ["60 fps", "720p", "Auto"], "Auto's limit, last");
    let mut ui = live(&vm);
    ui.click("1080p").unwrap();
    assert_eq!(asked(ui), [Intent::Resolution(Resolution::P1080)]);
    let mut ui = live(&vm);
    ui.click("1440p").unwrap();
    assert_eq!(asked(ui), [Intent::Resolution(Resolution::P1440)]);
    let mut ui = live(&vm);
    ui.click("8 Mbit/s").unwrap();
    assert_eq!(asked(ui), [Intent::Bitrate(Some(8_000))]);
}

#[test]
fn its_last_rows_change_what_is_shared_and_stop() {
    let vm = built(samples::live());
    let mut ui = live(&vm);
    ui.click("Change Windows").unwrap();
    assert_eq!(asked(ui).last(), Some(&Intent::Change));
    let mut ui = live(&vm);
    ui.click("Stop Streaming").unwrap();
    assert_eq!(asked(ui).last(), Some(&Intent::Stop));
    let starting = built(samples::live_starting());
    let mut ui = live(&starting);
    ui.click("Change Windows").unwrap();
    assert!(!asked(ui).contains(&Intent::Change));
}

#[test]
fn its_keys_go_to_the_menu_and_escape_only_while_a_submenu_shows() {
    let vm = built(samples::live());
    let mut ui = live(&vm);
    for key in [Named::ArrowDown, Named::ArrowRight, Named::Escape] {
        ui.tap_key(key);
    }
    assert_eq!(
        asked(ui),
        [Intent::Key(Key::Down), Intent::Key(Key::Right)],
        "Escape is the layer's"
    );
    let open = built(samples::live_quality());
    let mut ui = live(&open);
    ui.tap_key(Named::Escape);
    assert_eq!(asked(ui), [Intent::Key(Key::Escape)]);
}
