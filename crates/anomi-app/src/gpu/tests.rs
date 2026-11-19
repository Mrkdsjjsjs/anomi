use iced_wgpu::adapter::{self, Choice as Wanted, Info, Kind, Path};
use iced_wgpu::lost::Cause;
use yapfast_core::adapters::{Adapter, Vendor};

use super::fail::Failure;
use super::plan::{Why, decide, died};
use super::{Choice, Override, Row, adapter_id, drivers, list, screen};

/// This repository's owner's desktop as sysfs lists it: the RTX 3080 drives
/// the monitor, the Radeon 610M in the processor drives none.
fn desktop() -> Vec<Adapter> {
    vec![
        card(
            "pci:0000:01:00.0",
            "NVIDIA GeForce RTX 3080",
            Vendor::Nvidia,
            0x2216,
            "nvidia",
            false,
        ),
        card(
            "pci:0000:73:00.0",
            "AMD Radeon 610M",
            Vendor::Amd,
            0x13c0,
            "amdgpu",
            true,
        ),
    ]
}

fn card(
    key: &str,
    name: &str,
    vendor: Vendor,
    device: u16,
    driver: &str,
    integrated: bool,
) -> Adapter {
    Adapter {
        key: key.to_owned(),
        vendor,
        device,
        name: name.to_owned(),
        integrated,
        drives_display: !integrated,
        render_node: None,
        kernel_driver: driver.to_owned(),
        suspended: false,
    }
}

fn every(_card: &Adapter) -> bool {
    true
}

/// As this desktop is now: NVIDIA's Vulkan driver alone, no `vulkan-radeon`.
fn nvidia_only(card: &Adapter) -> bool {
    card.kernel_driver == "nvidia"
}

#[test]
fn a_choice_reads_and_writes_as_layout_json_keeps_it() {
    assert_eq!(Choice::parse(None), Choice::Auto);
    assert_eq!(Choice::parse(Some("")), Choice::Auto);
    assert_eq!(Choice::parse(Some("auto")), Choice::Auto);
    assert_eq!(Choice::parse(Some("processor")), Choice::Processor);
    let radeon = Choice::parse(Some("pci:0000:73:00.0"));
    assert_eq!(radeon, Choice::Adapter("pci:0000:73:00.0".into()));
    for choice in [Choice::Auto, Choice::Processor, radeon] {
        assert_eq!(Choice::parse(choice.saved().as_deref()), choice);
    }
}

#[test]
fn automatic_takes_the_integrated_graphics_and_never_the_discrete_card() {
    let plan = decide(&Choice::Auto, &desktop(), every, 0, false);
    let target = plan.target.expect("the Radeon draws");
    assert_eq!(
        (target.wanted, target.id.as_str()),
        (Wanted::Auto, "pci:0000:73:00.0")
    );
    assert_eq!(
        plan.drivers,
        ["*radeon*", "*amd*"],
        "NVIDIA's driver never loads"
    );
    let cards = desktop();
    let plan = decide(&Choice::Auto, &cards[..1], every, 0, false);
    assert_eq!((plan.target, plan.why), (None, Some(Why::NoIntegrated)));
    assert!(!plan.notice, "the rule, not a failure");
}

#[test]
fn without_vulkan_radeon_automatic_draws_on_the_processor_and_says_why() {
    let plan = decide(&Choice::Auto, &desktop(), nvidia_only, 0, false);
    assert_eq!(plan.target, None);
    assert_eq!(plan.why, Some(Why::NoDriver));
    assert!(!plan.notice);
    assert!(
        plan.why
            .is_some_and(|why| why.text().contains("Vulkan driver"))
    );
}

#[test]
fn a_card_chosen_by_hand_draws_and_loads_its_own_driver_alone() {
    let nvidia = Choice::Adapter("pci:0000:01:00.0".into());
    let plan = decide(&nvidia, &desktop(), every, 0, false);
    let target = plan.target.expect("the 3080 draws");
    assert_eq!(target.wanted, Wanted::Bus("0000:01:00.0".into()));
    assert_eq!(plan.drivers, ["*nvidia*"]);
    let radeon = Choice::Adapter("pci:0000:73:00.0".into());
    let plan = decide(&radeon, &desktop(), nvidia_only, 0, false);
    assert_eq!(plan.why, Some(Why::NoDriver));
    assert!(plan.notice, "chosen by hand, the window says why");
    let gone = Choice::Adapter("pci:0000:02:00.0".into());
    let plan = decide(&gone, &desktop(), every, 0, false);
    assert_eq!(
        (plan.target, plan.why, plan.notice),
        (None, Some(Why::Gone), true)
    );
}

#[test]
fn three_failures_in_a_row_keep_the_processor_unless_the_command_line_asks() {
    let plan = decide(&Choice::Auto, &desktop(), every, 2, false);
    assert!(plan.target.is_some(), "two are tried again");
    let plan = decide(&Choice::Auto, &desktop(), every, 3, false);
    assert_eq!(plan.why, Some(Why::Stopped));
    assert_eq!(
        plan.drivers,
        ["*radeon*", "*amd*"],
        "for a choice made while it runs"
    );
    assert!(
        decide(&Choice::Auto, &desktop(), every, 3, true)
            .target
            .is_some()
    );
}

#[test]
fn the_processor_lets_automatics_driver_load_for_a_switch() {
    let plan = decide(&Choice::Processor, &desktop(), every, 0, false);
    assert_eq!((plan.target, plan.why), (None, None));
    assert_eq!(plan.drivers, ["*radeon*", "*amd*"]);
    let plan = decide(&Choice::Processor, &desktop()[..1], every, 0, false);
    assert_eq!(
        plan.drivers,
        ["*nvidia*"],
        "without integrated graphics, every card's"
    );
}

#[test]
fn the_command_line_overrides_for_one_run() {
    let kept = Choice::Adapter("pci:0000:01:00.0".into());
    assert_eq!(Override::Gpu.choice(&kept), kept);
    assert_eq!(Override::Gpu.choice(&Choice::Processor), Choice::Auto);
    assert_eq!(Override::Software.choice(&kept), Choice::Processor);
    assert_eq!(Override::Auto.choice(&kept), Choice::Auto);
    assert_eq!(adapter_id("0000:73:00.0"), "pci:0000:73:00.0");
    assert_eq!(adapter_id(" pci:0000:01:00.0 "), "pci:0000:01:00.0");
}

fn vulkan(name: &str, bus: &str, vendor: u32, device: u32, kind: Kind) -> Info {
    Info {
        name: name.to_owned(),
        bus: bus.to_owned(),
        vendor,
        device,
        kind,
        driver: String::new(),
        driver_info: String::new(),
        direct: None,
    }
}

#[test]
fn the_childs_list_matches_vulkans_adapters_to_the_systems_by_address() {
    let nvidia = vulkan(
        "NVIDIA GeForce RTX 3080",
        "0000:01:00.0",
        0x10de,
        0x2216,
        Kind::Discrete,
    );
    let rows = list::merge(&desktop(), std::slice::from_ref(&nvidia));
    let usable: Vec<(&str, bool)> = rows
        .iter()
        .map(|row| (row.name.as_str(), row.usable))
        .collect();
    assert_eq!(
        usable,
        [
            ("NVIDIA GeForce RTX 3080", true),
            ("AMD Radeon 610M", false)
        ]
    );
    let radeon = vulkan(
        "AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)",
        "",
        0x1002,
        0x13c0,
        Kind::Integrated,
    );
    let rows = list::merge(&desktop(), &[nvidia, radeon]);
    assert!(
        rows.iter().all(|row| row.usable),
        "by vendor and device without an address"
    );
    assert_eq!(rows[1].name, "AMD Radeon 610M", "the system's name");
    let unlisted = vulkan("Virtual", "0000:05:00.0", 0x1af4, 0x1050, Kind::Other);
    let rows = list::merge(&[], &[unlisted]);
    let row = Row {
        id: "pci:0000:05:00.0".into(),
        name: "Virtual".into(),
        integrated: false,
        drives_display: false,
        usable: true,
    };
    assert_eq!(rows, [row]);
}

#[test]
fn the_loaders_globs_pick_each_family_and_never_lavapipe() {
    let cards = desktop();
    let radeon = &cards[1];
    let names = [
        "radeon_icd.x86_64.json",
        "amd_icd64.json",
        "nvidia_icd.json",
        "lvp_icd.x86_64.json",
    ];
    let picked: Vec<&str> = (names.iter().copied())
        .filter(|name| {
            drivers::globs(radeon)
                .iter()
                .any(|glob| drivers::matches(glob, name))
        })
        .collect();
    assert_eq!(picked, ["radeon_icd.x86_64.json", "amd_icd64.json"]);
    assert!(drivers::matches("*intel*", "intel_hasvk_icd.x86_64.json"));
    assert!(drivers::matches("nvidia*", "nvidia_icd.json"));
    assert!(!drivers::matches("*nvidia", "nvidia_icd.json"));
    assert!(drivers::globs(&card("luid:1", "x", Vendor::Other, 1, "", false)).is_empty());
}

#[test]
fn a_failure_says_what_failed_and_what_comes_next() {
    let why = Why::Failed {
        failure: Failure::Lost(Cause::DeviceLost),
        name: "AMD Radeon 610M".into(),
        last: false,
    };
    assert_eq!(
        why.text(),
        "AMD Radeon 610M stopped working. It's tried again at the next start."
    );
    let last = Why::Failed {
        failure: Failure::Start,
        name: "AMD Radeon 610M".into(),
        last: true,
    };
    assert!(last.text().contains("three failures in a row"));
}

/// The Radeon as Vulkan names it on this desktop.
fn radeon() -> Info {
    vulkan(
        "AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)",
        "0000:73:00.0",
        0x1002,
        0x13c0,
        Kind::Integrated,
    )
}

#[test]
fn on_this_desktop_automatic_draws_on_the_radeon_through_shared_memory() {
    let cards = desktop();
    let display = screen::decide(None, &cards);
    assert_eq!(
        display.as_deref(),
        Some("0000:01:00.0"),
        "the monitor's card"
    );
    let told = screen::decide(Some("0000:01:00.0"), &cards);
    assert_eq!(told, display, "as mutter's feedback names it");
    let target = decide(&Choice::Auto, &cards, every, 0, false).target;
    assert_eq!(
        target.map(|target| target.id).as_deref(),
        Some("pci:0000:73:00.0")
    );
    assert_eq!(adapter::path(&radeon(), display.as_deref()), Path::Shared);
}

#[test]
fn the_card_the_monitor_is_on_draws_straight_into_the_window() {
    let nvidia = vulkan(
        "NVIDIA GeForce RTX 3080",
        "0000:01:00.0",
        0x10de,
        0x2216,
        Kind::Discrete,
    );
    let display = screen::decide(None, &desktop());
    assert_eq!(adapter::path(&nvidia, display.as_deref()), Path::Direct);
}

#[test]
fn a_laptop_with_one_adapter_draws_straight_into_the_window() {
    let mut intel = card(
        "pci:0000:00:02.0",
        "Intel Iris Xe Graphics",
        Vendor::Intel,
        0x9a49,
        "i915",
        true,
    );
    intel.drives_display = true;
    let info = vulkan(
        "Intel(R) Xe Graphics",
        "0000:00:02.0",
        0x8086,
        0x9a49,
        Kind::Integrated,
    );
    let display = screen::decide(None, std::slice::from_ref(&intel));
    assert_eq!(adapter::path(&info, display.as_deref()), Path::Direct);
    intel.drives_display = false;
    let shut = screen::decide(None, std::slice::from_ref(&intel));
    assert_eq!(shut, display, "its lid shut, the only adapter still");
}

#[test]
fn not_knowing_the_desktops_adapter_reads_every_adapter_back() {
    let mut cards = desktop();
    cards[1].drives_display = true;
    assert_eq!(screen::decide(None, &cards), None, "monitors on both");
    assert_eq!(adapter::path(&radeon(), None), Path::Shared);
    let told = screen::decide(Some("0000:73:00.0"), &cards);
    assert_eq!(adapter::path(&radeon(), told.as_deref()), Path::Direct);
    assert_eq!(screen::decide(None, &[]), None);
}

#[test]
fn the_compositors_device_reads_as_sysfs_writes_it() {
    assert!(screen::is_pci_address("0000:01:00.0"));
    assert!(screen::is_pci_address("0000:73:00.0"));
    assert!(!screen::is_pci_address("card1"));
    assert!(!screen::is_pci_address("0000:01:00"));
    assert!(!screen::is_pci_address("0000-01-00.0"));
    let device: u64 = (226 << 8) | 1;
    assert_eq!(screen::device_number(&device.to_ne_bytes()), Some(device));
    assert_eq!(screen::device_number(&[1, 2, 3]), None);
}

#[test]
fn a_start_that_died_on_an_adapter_leaves_the_processor_until_chosen_again() {
    let radeon = "pci:0000:73:00.0";
    let mut plan = decide(&Choice::Auto, &desktop(), every, 0, false);
    died(&mut plan, Some(radeon), false);
    assert_eq!(plan.target, None);
    assert!(plan.notice, "the window says why");
    assert!(matches!(&plan.why, Some(Why::Died(name)) if name == "AMD Radeon 610M"));
    assert!(
        plan.why
            .is_some_and(|why| why.text().contains("chosen again"))
    );
    assert_eq!(
        plan.drivers,
        ["*radeon*", "*amd*"],
        "for a choice while it runs"
    );
    let mut forced = decide(&Choice::Auto, &desktop(), every, 0, true);
    died(&mut forced, Some(radeon), true);
    assert!(forced.target.is_some(), "the command line still tries it");
    let mut other = decide(&Choice::Auto, &desktop(), every, 0, false);
    died(&mut other, Some("pci:0000:01:00.0"), false);
    assert!(other.target.is_some(), "another adapter died");
}
