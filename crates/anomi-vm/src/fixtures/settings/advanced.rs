//! Samples of the Advanced settings page for tests and shots, after the fake
//! Discord's data: task S10's, and Optimization on this repository's
//! owner's machine, a Ryzen 9 9950X3D with a Radeon 610M and an RTX 3080.

use yapfast_lang::tr;

use crate::Vm;
use crate::fixtures::settings::frame::settings;
use crate::settings::advanced::{Advanced, Core, CoreGroup, CpuMode};
use crate::settings::frame::Page;

/// The two chiplets' physical cores, the second's ticked.
fn chiplets() -> Vec<CoreGroup> {
    let group = |title: String, cores: std::ops::Range<u32>, chosen: bool| CoreGroup {
        title,
        cores: cores
            .map(|id| Core {
                id,
                label: id.to_string(),
                chosen,
            })
            .collect(),
    };
    vec![
        group(tr!("Chiplet {} · L3 {} MB", 1, 96), 0..8, false),
        group(tr!("Chiplet {} · L3 {} MB", 2, 32), 8..16, true),
    ]
}

/// The golden screenshot's: no system tray, link handling and autostart
/// offered; Auto on the second chiplet and on the integrated graphics.
pub fn advanced() -> Advanced {
    Advanced {
        close_to_tray: true,
        multiple_windows: false,
        developer_mode: false,
        start_on_sign_in: false,
        discord_links: false,
        tray_available: false,
        autostart_available: true,
        discord_links_available: true,
        cpu_mode: CpuMode::Auto.index(),
        // As `yapfast_core::cpu::describe` says it.
        cpu_line: format!(
            "{} {}",
            tr!("Chiplets: {0}, cores: {1}, threads: {2}.", 2, 16, 32),
            tr!(
                "Runs on cores {0} (CPUs {1}): the chiplet with the smaller cache, {2} MB.",
                "8-15",
                "8-15, 24-31",
                32
            )
        ),
        core_groups: chiplets(),
        encoders: vec![
            tr!("Auto").into(),
            "NVIDIA GeForce RTX 3080".into(),
            "AMD Radeon 610M".into(),
            tr!("Processor").into(),
        ],
        // In PCI order, as the system lists them.
        encoder_keys: vec![
            "auto".into(),
            "pci:0000:01:00.0".into(),
            "pci:0000:73:00.0".into(),
            "software".into(),
        ],
        encoder: 0,
        encoder_line: tr!(
            "Auto takes the integrated graphics first, then a discrete card, then the processor. Now it's {}.",
            "AMD Radeon 610M"
        ),
    }
}

/// The window with the settings open on Advanced.
pub fn page() -> Vm {
    let mut vm = settings(Page::Advanced);
    vm.settings.advanced = advanced();
    vm
}

/// The cores chosen by hand, shown to tick; streams on the processor.
pub fn cores() -> Vm {
    let mut vm = page();
    let page = &mut vm.settings.advanced;
    page.cpu_mode = CpuMode::Chosen.index();
    page.encoder = 3;
    page.encoder_line = tr!(
        "The processor encodes with x264: it takes processor time, and H.265 and AV1 aren't offered."
    )
    .into();
    vm
}

/// A tray, but no autostart and no link handling: those switches go.
pub fn bare() -> Vm {
    let mut vm = page();
    let page = &mut vm.settings.advanced;
    page.tray_available = true;
    page.autostart_available = false;
    page.discord_links_available = false;
    vm
}
