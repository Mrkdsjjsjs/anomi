//! The Game activity settings page: task S08's tests, driving
//! `anomi_settings::games::view` over its fixtures in the simulator
//! (`super::support`).

use iced::Widget as _;
use iced_test::simulator::Simulator;

use anomi_settings::games::view;
use anomi_vm::fixtures::settings::games as sample;
use anomi_vm::settings::games::{Games, Intent};

use super::support::{SIZE, asked, front, settings};

fn simulate(page: &Games) -> Simulator<'_, Intent, anomi_theme::Theme, anomi_theme::Renderer> {
    Simulator::with_size(settings(), SIZE, view(page, front()).boxed())
}

#[test]
fn the_switch_asks_to_stop_sharing() {
    let page = sample::games();
    let mut ui = simulate(&page);
    ui.click("Share the game you play")
        .expect("the switch is there");
    assert_eq!(asked(ui), [Intent::Share(false)]);
}

#[test]
fn the_switch_is_locked_without_an_account() {
    let page = sample::unlinked();
    let mut ui = simulate(&page);
    let _ = ui.click("Share the game you play");
    assert!(asked(ui).is_empty());
}

#[test]
fn each_game_can_be_edited_and_removed() {
    let page = sample::custom();
    let mut ui = simulate(&page);
    assert!(ui.find("Blockhead").is_ok());
    ui.click("Remove").expect("a button each");
    assert_eq!(asked(ui), [Intent::Remove(0)]);
}

#[test]
fn adding_waits_for_both_fields() {
    let page = sample::games();
    let mut ui = simulate(&page);
    let _ = ui.click("Add custom game");
    assert!(asked(ui).is_empty());
}

#[test]
fn add_game_opens_the_picker_of_discords_games() {
    let page = sample::games();
    let mut ui = simulate(&page);
    ui.click("Add game").expect("the picker's button");
    assert_eq!(asked(ui), [Intent::OpenPicker]);
}

#[test]
fn a_game_from_discords_list_switches_off_and_goes() {
    let page = sample::added();
    let mut ui = simulate(&page);
    assert!(
        ui.find("Without a program: shows until you turn it off")
            .is_ok(),
        "one shown without a program says so"
    );
    ui.click("Factorio").expect("its switch");
    ui.click("Remove").expect("a button each");
    assert_eq!(
        asked(ui),
        [Intent::ShowAdded(0, false), Intent::RemoveAdded(0)]
    );
}

#[test]
fn the_picker_searches_as_typed() {
    let page = sample::downloading();
    let mut ui = simulate(&page);
    assert!(
        ui.find("Discord's list of games isn't downloaded yet. Downloading it...")
            .is_ok()
    );
    ui.click("Search Discord's games").expect("the search");
    ui.typewrite("f");
    assert_eq!(asked(ui), [Intent::Search("f".into())]);
}

#[test]
fn a_game_found_by_another_name_is_chosen_by_it_too() {
    let page = sample::picker();
    let mut ui = simulate(&page);
    assert!(
        ui.find("These are the best matches. Type more of the name to find others.")
            .is_ok()
    );
    ui.click("Also known as Dwarves: The Deep Factory")
        .expect("the second match");
    ui.click("Cancel").expect("the picker closes");
    assert_eq!(asked(ui), [Intent::PickGame(1), Intent::ClosePicker]);
}

#[test]
fn a_game_chosen_is_tied_to_a_program_or_none() {
    let page = sample::programs();
    let mut ui = simulate(&page);
    ui.click("Game.exe").expect("a program running");
    ui.click("Without a program").expect("shown without one");
    ui.click("Look again").expect("listed again");
    ui.click("Back").expect("back to the search");
    assert_eq!(
        asked(ui),
        [
            Intent::PickProgram(1),
            Intent::WithoutProgram,
            Intent::ListPrograms,
            Intent::BackToGames
        ]
    );
}

#[test]
fn a_failed_download_can_be_tried_again() {
    let page = sample::failed();
    let mut ui = simulate(&page);
    assert!(
        ui.find("Couldn't download Discord's list of games.")
            .is_ok()
    );
    ui.click("Retry").expect("Retry");
    assert_eq!(asked(ui), [Intent::RetryList]);
}

#[test]
fn save_asks_to_keep_the_edit() {
    let page = sample::custom();
    let mut ui = simulate(&page);
    ui.click("Save game").expect("a game is being edited");
    assert_eq!(asked(ui), [Intent::Save]);
}

#[test]
fn cancel_shows_only_while_editing() {
    assert!(simulate(&sample::games()).find("Cancel").is_err());
    let page = sample::custom();
    let mut ui = simulate(&page);
    ui.click("Cancel").expect("an edit can be left");
    assert_eq!(asked(ui), [Intent::Cancel]);
}

#[test]
fn typing_reports_the_whole_text() {
    let page = sample::games();
    let mut ui = simulate(&page);
    ui.click("Executable name or full path")
        .expect("the executable's field");
    ui.typewrite("g");
    ui.click("Text after Playing (up to 128 characters)")
        .expect("the text's field");
    ui.typewrite("x");
    assert_eq!(
        asked(ui),
        [Intent::Executable("g".into()), Intent::Text("x".into())]
    );
}

#[test]
fn edit_names_the_game() {
    let page = sample::custom();
    let mut ui = simulate(&page);
    ui.click("Edit").expect("a button each");
    assert_eq!(asked(ui), [Intent::Edit(0)]);
}

#[test]
fn a_refused_save_says_why() {
    let page = sample::refused();
    assert!(
        simulate(&page)
            .find("Enter an executable and a name of up to 128 characters, without line breaks.")
            .is_ok()
    );
}
