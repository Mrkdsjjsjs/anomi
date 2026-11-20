//! The Advanced settings page: switches for how the program runs. The Slint
//! side: `AdvancedSettings` (`ui/settings.slint:1881`), a column 16 px
//! apart; the page's title is the frame's bar's.
//!
//! Under the switches, Anomi's Optimization: which of the processor's
//! cores the program runs on, with a line saying what the processor is and
//! which cores it got, the physical cores to tick by chiplet while they are
//! chosen by hand; what draws the interface (`drawing`), which switches
//! at once and so has no restart line (the Slint app's "Render with the
//! GPU" changes nothing here); where streams encode, with what that
//! choice does now; "Unlocked bitrate", which lets a stream's limit be
//! typed past Discord's documented 9 Mbit/s, up to 50, saying what that
//! risks; and what the window keeps doing on screen but not focused, such
//! as on a second monitor beside a game: everything, what matters, or
//! nothing, with what "what matters" keeps going and, on a desktop that
//! can't say when a game covers the window, a line saying so and that the
//! window rests there until chosen otherwise.

mod drawing;

use iced::Length::Fill;
use iced::widget::{column, container, row, text};

use anomi_theme::prelude::*;
use anomi_theme::{catalog, typography};
use anomi_vm::settings::advanced::{Advanced, Background, CoreGroup, Intent};
use anomi_widgets::button::{Kind, action_button};
use yapfast_lang::tr;

use crate::kit::{choices, heading, switch, words};

/// Between the switches (`spacing: 16px`).
const SPACING: f32 = 16.0;
/// Between a heading and its control, and the lines under it.
const GROUP: f32 = 8.0;
/// Between the cores' buttons.
const CORE_GAP: f32 = 4.0;

/// Small grey text under a control.
fn note<'a>(words: &'a str) -> Element<'a, Intent> {
    typography::label(words).width(Fill).boxed()
}

/// A line the width of the page, in the theme's rule colour.
fn rule<'a>() -> Element<'a, Intent> {
    container("")
        .width(Fill)
        .height(1)
        .style(|theme: &Theme| container::Style {
            background: Some(theme.palette().line.into()),
            ..container::Style::default()
        })
        .boxed()
}

pub fn view(page: &Advanced, motion: Motion) -> impl View<Intent> + '_ {
    let mut parts: Vec<Element<'_, Intent>> = Vec::new();
    if page.autostart_available {
        parts.push(
            switch(tr!("Start when you sign in"), page.start_on_sign_in, motion)
                .description(tr!(
                    "Yapfast starts in the system tray when you sign in to your computer, so messages and calls reach you."
                ))
                .on_toggle(Intent::StartOnSignIn)
                .view(),
        );
    }
    if page.discord_links_available {
        parts.push(
            switch(tr!("Open Discord links in Yapfast"), page.discord_links, motion)
                .description(tr!(
                    "discord:// links from other apps and shortcuts open here instead of Discord. Turning it off gives them back to the app that had them."
                ))
                .on_toggle(Intent::DiscordLinks)
                .view(),
        );
    }
    let tray = if page.tray_available {
        tr!(
            "Closing the window keeps Yapfast running in the system tray, with any call. Quit from the tray icon's menu."
        )
    } else {
        tr!("No system tray is running, so closing the window quits Yapfast.")
    };
    parts.push(
        switch(tr!("Close to tray"), page.close_to_tray, motion)
            .description(tray)
            .on_toggle(Intent::CloseToTray)
            .view(),
    );
    parts.push(
        switch(tr!("Allow multiple windows"), page.multiple_windows, motion)
            .description(tr!(
                "Starting Yapfast while it runs opens another Yapfast, with its own window and tray icon. Off, it brings the open window forward."
            ))
            .on_toggle(Intent::MultipleWindows)
            .view(),
    );
    parts.push(
        switch(tr!("Developer Mode"), page.developer_mode, motion)
            .description(tr!(
                "Menus offer Copy ID for servers, channels and people, which helps when you write apps that use the Discord API."
            ))
            .on_toggle(Intent::DeveloperMode)
            .view(),
    );
    parts.push(rule());
    parts.push(heading(tr!("OPTIMIZATION")).boxed());
    parts.push(cores(page, motion));
    parts.push(drawing::view(page, motion));
    parts.push(encoders(page, motion));
    parts.push(
        switch(tr!("Unlocked bitrate"), page.bitrate_unlocked, motion)
            .description(tr!(
                "Lets a stream's bitrate limit be set above 9 Mbit/s, up to 50. Discord documents 9 Mbit/s as the most, and was seen to take up to about 16 Mbit/s; more may break the stream: it can be cut off, lose packets or be dropped. A higher bitrate also needs an upload that fast, and loads the encoder and the people watching."
            ))
            .on_toggle(Intent::BitrateUnlocked)
            .view(),
    );
    parts.push(unfocused(page, motion));
    parts.push(
        text(tr!(
            "Icons from Solar by 480 Design (CC BY 4.0), many of them adapted, and from Lucide (ISC)."
        ))
        .size(typography::size::CAPTION)
        .style(catalog::text::faint)
        .boxed(),
    );
    column(parts).spacing(SPACING).width(Fill)
}

/// A control's name over it, as a switch's.
fn name<'a>(words: &'a str) -> Element<'a, Intent> {
    text(words)
        .size(typography::size::BODY)
        .style(catalog::text::primary)
        .boxed()
}

/// Which cores the program runs on: Auto, all of them or those ticked, and
/// what that gives on this processor.
fn cores(page: &Advanced, motion: Motion) -> Element<'_, Intent> {
    let modes = words(&[tr!("Auto"), tr!("All cores"), tr!("Choose cores")]);
    let mut parts = vec![
        name(tr!("CPU load distribution")),
        choices(modes, page.cpu_mode, motion)
            .wrap(true)
            .on_pick(Intent::CpuMode)
            .view(),
        note(&page.cpu_line),
    ];
    if page.choosing() {
        parts.extend(page.core_groups.iter().map(|group| chips(group, motion)));
        parts.push(note(tr!(
            "Yapfast and its streams run only on the cores ticked, each with its second thread. The rest stay free for games."
        )));
    }
    column(parts).spacing(GROUP).width(Fill).boxed()
}

/// A chiplet's cores as buttons to tick, under its name.
fn chips(group: &CoreGroup, motion: Motion) -> Element<'_, Intent> {
    let buttons = group.cores.iter().map(|core| {
        let kind = if core.chosen {
            Kind::Primary
        } else {
            Kind::Secondary
        };
        action_button(&core.label, kind, motion)
            .small()
            .on_press(Intent::Core(core.id))
            .boxed()
    });
    column![
        typography::label(&group.title),
        row(buttons)
            .spacing(CORE_GAP)
            .wrap()
            .vertical_spacing(CORE_GAP),
    ]
    .spacing(CORE_GAP)
    .width(Fill)
    .boxed()
}

/// Where streams encode: Auto, a graphics card, or the processor.
fn encoders(page: &Advanced, motion: Motion) -> Element<'_, Intent> {
    let keys = &page.encoder_keys;
    let mut parts = vec![
        name(tr!("Encode streams on")),
        choices(&page.encoders, page.encoder, motion)
            .wrap(true)
            .on_pick(move |at| Intent::Encoder(keys.get(at).cloned().unwrap_or_default()))
            .view(),
    ];
    if !page.encoder_line.is_empty() {
        parts.push(note(&page.encoder_line));
    }
    column(parts).spacing(GROUP).width(Fill).boxed()
}

/// What the window keeps doing on screen but not focused, the choice in
/// force raised; what "what matters" keeps going; and whether the desktop
/// says when the window is hidden, where nothing goes under any choice.
fn unfocused(page: &Advanced, motion: Motion) -> Element<'_, Intent> {
    let options = words(&[tr!("Everything"), tr!("What matters"), tr!("Rest")]);
    let hidden = if page.exact {
        tr!(
            "Covered by other windows, minimised or in the tray, the window does nothing, whatever is chosen."
        )
    } else {
        tr!(
            "This system doesn't tell when a game or another window covers this one, and Everything and What matters keep going even behind them. So Rest is the default here: the window rests as soon as it loses the focus."
        )
    };
    column![
        name(tr!("When the window is on screen but not focused")),
        choices(options, page.background.index(), motion)
            .wrap(true)
            .on_pick(|at| Intent::Background(Background::at(at)))
            .view(),
        note(tr!(
            "With What matters, video, cameras, streams, who's typing and times keep going, while animations, GIFs and members' statuses wait until you come back to the window."
        )),
        note(hidden),
    ]
    .spacing(GROUP)
    .width(Fill)
    .boxed()
}

/// Its shots for `anomi-shots`: `settings/advanced.png` as the Slint app
/// showed it, with Optimization under it, and without the system's extras;
/// the cores chosen by hand, also in the small window; the interface drawn
/// on the integrated graphics, and on the processor after the card chosen
/// failed (without the system's extras, so the rows show in the window);
/// "Unlocked bitrate" on, in a window tall enough to show it at the
/// page's end; and, in windows taller still, what an unfocused window on
/// screen keeps doing where the desktop says when a game covers it, and, as
/// wide as the small window, on one that can't, as on GNOME's.
#[cfg(feature = "shots")]
pub fn shots() -> Vec<crate::shots::Shot> {
    use anomi_vm::fixtures::settings::advanced::{
        bare, coarse, cores, fell_back, gpu, page, unlocked,
    };

    use crate::shots::{GOLDEN, SMALL, Shot};

    [
        ("advanced", page as fn() -> anomi_vm::Vm, GOLDEN),
        ("advanced_bare", bare, GOLDEN),
        ("advanced_cores", cores, GOLDEN),
        ("advanced_cores_small", cores, SMALL),
        ("advanced_gpu", gpu, GOLDEN),
        ("advanced_fell_back", fell_back, GOLDEN),
        ("advanced_unlocked", unlocked, (1400.0, 1400.0)),
        ("advanced_unfocused", bare, (1400.0, 1700.0)),
        ("advanced_unfocused_coarse", coarse, (SMALL.0, 1900.0)),
    ]
    .into_iter()
    .map(|(name, vm, size)| Shot {
        area: "settings",
        name,
        size,
        theme: "bonfire",
        vm,
    })
    .collect()
}
