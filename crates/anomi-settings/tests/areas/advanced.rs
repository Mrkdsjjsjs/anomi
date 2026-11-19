//! The Advanced settings page: task S10's tests, driving
//! `anomi_settings::advanced::view` over its fixtures in the simulator
//! (`super::support`), and Optimization's choices.

use iced::Widget as _;
use iced_test::simulator::Simulator;

use anomi_settings::advanced::view;
use anomi_vm::fixtures::settings::advanced as sample;
use anomi_vm::settings::advanced::{Advanced, CpuMode, Intent};

use super::support::{SIZE, asked, front, settings};

/// With Optimization the page runs past the window's height, where the
/// frame scrolls it: the simulator gets room for all of it.
fn simulate(page: &Advanced) -> Simulator<'_, Intent, anomi_theme::Theme, anomi_theme::Renderer> {
    let tall = iced::Size::new(SIZE.width, 1600.0);
    Simulator::with_size(settings(), tall, view(page, front()).boxed())
}

#[test]
fn a_switch_asks_for_its_opposite() {
    let page = sample::advanced();
    let mut ui = simulate(&page);
    for name in [
        "Start when you sign in",
        "Open Discord links in Yapfast",
        "Close to tray",
        "Allow multiple windows",
        "Developer Mode",
    ] {
        ui.click(name).expect("the switch is there");
    }
    assert_eq!(
        asked(ui),
        [
            Intent::StartOnSignIn(true),
            Intent::DiscordLinks(true),
            Intent::CloseToTray(false),
            Intent::MultipleWindows(true),
            Intent::DeveloperMode(true),
        ]
    );
}

#[test]
fn the_gpu_rendering_switch_is_a_line_here() {
    let page = sample::advanced();
    let mut ui = simulate(&page);
    assert!(ui.find("Render with the GPU").is_err());
    assert!(ui.find("Restart Yapfast").is_err());
    assert!(
        ui.find("The interface draws on the processor and leaves the graphics cards free.")
            .is_ok()
    );
}

#[test]
fn optimization_chooses_how_the_cores_are_spread_and_says_so() {
    let page = sample::advanced();
    let mut ui = simulate(&page);
    assert!(ui.find(page.cpu_line.as_str()).is_ok(), "the line under it");
    assert!(ui.find("15").is_err(), "the cores show only to choose");
    ui.click("Choose cores").expect("the choice is there");
    assert_eq!(asked(ui), [Intent::CpuMode(CpuMode::Chosen.index())]);
    let choosing = Advanced {
        cpu_mode: CpuMode::Chosen.index(),
        ..sample::advanced()
    };
    let mut ui = simulate(&choosing);
    assert!(ui.find("Chiplet 1 · L3 96 MB").is_ok());
    ui.click("3").expect("a core of the first chiplet");
    ui.click("12").expect("a core of the second");
    assert_eq!(asked(ui), [Intent::Core(3), Intent::Core(12)]);
}

#[test]
fn optimization_chooses_where_streams_encode_by_key() {
    let page = sample::advanced();
    let mut ui = simulate(&page);
    assert!(ui.find(page.encoder_line.as_str()).is_ok());
    ui.click("NVIDIA GeForce RTX 3080")
        .expect("each card by name");
    ui.click("Processor").expect("and the processor");
    assert_eq!(
        asked(ui),
        [
            Intent::Encoder("pci:0000:01:00.0".into()),
            Intent::Encoder("software".into()),
        ]
    );
}

#[test]
fn switches_the_system_lacks_are_left_out() {
    let page = Advanced {
        autostart_available: false,
        discord_links_available: false,
        ..sample::advanced()
    };
    let mut ui = simulate(&page);
    assert!(ui.find("Start when you sign in").is_err());
    assert!(ui.find("Open Discord links in Yapfast").is_err());
}
