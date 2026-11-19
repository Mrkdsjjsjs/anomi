//! The adapter list and the kind of computer, read from copies of `/sys`:
//! the owner's computer as read on 19 November 2026 (a Radeon 610M in the
//! processor beside an RTX 3080 that drives the monitor, on a desktop), a
//! laptop's, and desktops with one card. Each is written to a folder as
//! sysfs lays it out, with folders where sysfs has links to them.

use std::{fs, path::Path};

use yapfast_lang::tr;

use super::{
    Adapter, EncoderApi, Machine, Signals, Vendor, auto_encoder, chassis_from_smbios, classify,
    cuda_index, encoder_order, find, linux,
};

/// The lines of `pci.ids` these computers' adapters are on, as hwdata
/// writes them, and a few around them.
const PCI_IDS: &str = "\
# List of PCI ID's
0014  Loongson Technology LLC
\t7a00  7A1000 Chipset Hyper Transport Bridge Controller
1002  Advanced Micro Devices, Inc. [AMD/ATI]
\t13c0  Granite Ridge [Radeon 610M]
\t\t1458 d000  Granite Ridge [Radeon Graphics]
\t1638  Cezanne [Radeon Vega Series / Radeon Vega Mobile Series]
10de  NVIDIA Corporation
\t2216  GA102 [GeForce RTX 3080 Lite Hash Rate]
\t25a0  GA107M [GeForce RTX 3050 Ti Mobile]
1af4  Red Hat, Inc.
\t1050  Virtio 1.0 GPU
8086  Intel Corporation
\t9a49  TigerLake-LP GT2 [Iris Xe Graphics]
ffff  Illegal Vendor ID

# List of known device classes, subclasses and programming interfaces
C 03  Display controller
\t00  VGA compatible controller
";

/// Writes `text` at `path` under `root`, making the folders on the way.
fn put(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().expect("a folder")).expect("folders made");
    fs::write(path, text).expect("written");
}

/// A card as `/sys/class/drm` and `/dev/dri` show it.
struct Card<'a> {
    number: u32,
    slot: &'a str,
    /// `PCI_ID` as the kernel writes it: `1002:13C0`.
    id: &'a str,
    driver: &'a str,
    /// `power/runtime_status`: `active` or `suspended`.
    runtime: &'a str,
    /// amdgpu's `mem_info_vram_total`.
    vram: Option<u64>,
    /// Its connectors, `DP-3`, and their status.
    connectors: &'a [(&'a str, &'a str)],
    render: u32,
    /// Whether udev made its `/dev/dri/by-path` link.
    by_path: bool,
}

fn card(root: &Path, card: &Card) {
    let device = format!("sys/class/drm/card{}/device", card.number);
    put(
        root,
        &format!("{device}/uevent"),
        &format!(
            "DRIVER={}\nPCI_CLASS=30000\nPCI_ID={}\nPCI_SUBSYS_ID=1458:D000\nPCI_SLOT_NAME={}\n",
            card.driver, card.id, card.slot
        ),
    );
    put(
        root,
        &format!("{device}/power/runtime_status"),
        &format!("{}\n", card.runtime),
    );
    if let Some(vram) = card.vram {
        put(
            root,
            &format!("{device}/mem_info_vram_total"),
            &format!("{vram}\n"),
        );
    }
    let render = format!("renderD{}", card.render);
    put(root, &format!("{device}/drm/{render}/dev"), "226:128\n");
    put(root, &format!("sys/class/drm/{render}/dev"), "226:128\n");
    for (connector, status) in card.connectors {
        put(
            root,
            &format!("sys/class/drm/card{}-{connector}/status", card.number),
            &format!("{status}\n"),
        );
    }
    put(root, &format!("dev/dri/{render}"), "");
    if card.by_path {
        let link = root.join(format!("dev/dri/by-path/pci-{}-render", card.slot));
        fs::create_dir_all(link.parent().expect("a folder")).expect("folders made");
        std::os::unix::fs::symlink(format!("../{render}"), link).expect("linked");
    }
}

/// The owner's computer: a Ryzen 9 9950X3D's Radeon 610M with nothing
/// plugged in, and an RTX 3080 with the monitor on DP-3.
fn owners_computer(root: &Path) {
    card(
        root,
        &Card {
            number: 0,
            slot: "0000:73:00.0",
            id: "1002:13C0",
            driver: "amdgpu",
            runtime: "active",
            vram: Some(536_870_912),
            connectors: &[
                ("DP-1", "disconnected"),
                ("DP-2", "disconnected"),
                ("HDMI-A-1", "disconnected"),
                ("HDMI-A-2", "disconnected"),
                ("Writeback-1", "unknown"),
            ],
            render: 128,
            by_path: true,
        },
    );
    card(
        root,
        &Card {
            number: 1,
            slot: "0000:01:00.0",
            id: "10DE:2216",
            driver: "nvidia",
            runtime: "active",
            vram: None,
            connectors: &[
                ("DP-3", "connected"),
                ("DP-4", "disconnected"),
                ("DP-5", "disconnected"),
                ("HDMI-A-3", "disconnected"),
            ],
            render: 129,
            by_path: true,
        },
    );
    put(root, "sys/class/drm/version", "drm 1.1.0 20060810\n");
    put(root, "sys/class/dmi/id/chassis_type", "3\n");
    // Its only battery is the wireless mouse's.
    put(
        root,
        "sys/class/power_supply/hidpp_battery_0/type",
        "Battery\n",
    );
    put(
        root,
        "sys/class/power_supply/hidpp_battery_0/scope",
        "Device\n",
    );
    put(root, "usr/share/hwdata/pci.ids", PCI_IDS);
}

/// A laptop: Intel's graphics in the processor drive the panel, and an
/// NVIDIA card sleeps until something wakes it.
fn laptop(root: &Path, card_sleeps: bool) {
    card(
        root,
        &Card {
            number: 0,
            slot: "0000:00:02.0",
            id: "8086:9A49",
            driver: "i915",
            runtime: "active",
            vram: None,
            connectors: &[("eDP-1", "connected"), ("HDMI-A-1", "disconnected")],
            render: 128,
            by_path: true,
        },
    );
    card(
        root,
        &Card {
            number: 1,
            slot: "0000:01:00.0",
            id: "10DE:25A0",
            driver: "nvidia",
            runtime: if card_sleeps { "suspended" } else { "active" },
            vram: None,
            connectors: &[],
            render: 129,
            by_path: true,
        },
    );
    put(root, "sys/class/dmi/id/chassis_type", "10\n");
    put(root, "sys/class/power_supply/BAT0/type", "Battery\n");
    put(root, "sys/class/power_supply/AC/type", "Mains\n");
    put(root, "usr/share/hwdata/pci.ids", PCI_IDS);
}

/// A desktop whose firmware turned the processor's graphics off, with one
/// NVIDIA card driven by `driver`, no udev links and no `pci.ids`; the
/// firmware's framebuffer, which isn't a PCI device, is still card0.
fn one_card(root: &Path, driver: &str) {
    put(
        root,
        "sys/class/drm/card0/device/uevent",
        "DRIVER=simple-framebuffer\nMODALIAS=platform:simple-framebuffer\n",
    );
    card(
        root,
        &Card {
            number: 1,
            slot: "0000:01:00.0",
            id: "10DE:2216",
            driver,
            runtime: "active",
            vram: None,
            connectors: &[("DP-1", "connected")],
            render: 128,
            by_path: false,
        },
    );
    put(root, "sys/class/dmi/id/chassis_type", "3\n");
}

fn adapter(key: &str, vendor: Vendor, driver: &str, integrated: bool, shows: bool) -> Adapter {
    Adapter {
        key: key.into(),
        vendor,
        device: 0,
        name: String::new(),
        integrated,
        drives_display: shows,
        render_node: None,
        kernel_driver: driver.into(),
        suspended: false,
    }
}

#[test]
fn the_owners_computer_encodes_on_the_radeon_in_its_processor() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    owners_computer(root);
    let adapters = linux::list(root, true);
    assert_eq!(
        adapters,
        [
            Adapter {
                key: "pci:0000:01:00.0".into(),
                vendor: Vendor::Nvidia,
                device: 0x2216,
                name: "NVIDIA GeForce RTX 3080 Lite Hash Rate".into(),
                integrated: false,
                drives_display: true,
                render_node: Some(root.join("dev/dri/by-path/pci-0000:01:00.0-render")),
                kernel_driver: "nvidia".into(),
                suspended: false,
            },
            Adapter {
                key: "pci:0000:73:00.0".into(),
                vendor: Vendor::Amd,
                device: 0x13c0,
                name: "AMD Radeon 610M".into(),
                // amdgpu can't be asked through a plain file: its 512 MiB tell.
                integrated: true,
                drives_display: false,
                render_node: Some(root.join("dev/dri/by-path/pci-0000:73:00.0-render")),
                kernel_driver: "amdgpu".into(),
                suspended: false,
            },
        ]
    );
    let (nvidia, radeon) = (&adapters[0], &adapters[1]);
    assert_eq!(
        linux::signals(root),
        Signals {
            built_in_panel: false,
            chassis: Some(3),
            system_battery: false,
        },
        "the mouse's battery isn't the computer's"
    );
    assert_eq!(linux::machine(root), Machine::Desktop);
    assert_eq!(radeon.encoder_api(), Some(EncoderApi::Vaapi));
    assert_eq!(nvidia.encoder_api(), Some(EncoderApi::Nvenc));
    assert_eq!(auto_encoder(&adapters, Machine::Desktop), Some(radeon));
    assert_eq!(encoder_order(&adapters, Machine::Desktop), [radeon, nvidia]);
    assert_eq!(find(&adapters, "pci:0000:73:00.0"), Some(radeon));
    assert_eq!(find(&adapters, "pci:0000:02:00.0"), None);
    assert_eq!(radeon.pci_address(), Some("0000:73:00.0"));
    assert_eq!(cuda_index(&adapters, nvidia), Some(0));
    assert_eq!(cuda_index(&adapters, radeon), None);
}

#[test]
fn a_laptop_encodes_on_its_processors_graphics_and_lets_its_card_sleep() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    laptop(root, true);
    let adapters = linux::list(root, true);
    let names: Vec<&str> = adapters
        .iter()
        .map(|adapter| adapter.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Intel Iris Xe Graphics",
            "NVIDIA GeForce RTX 3050 Ti Mobile"
        ]
    );
    let (intel, nvidia) = (&adapters[0], &adapters[1]);
    assert!(intel.integrated && intel.drives_display && !intel.suspended);
    assert!(!nvidia.integrated && !nvidia.drives_display && nvidia.suspended);
    assert_eq!(intel.encoder_api(), Some(EncoderApi::Vaapi));
    assert!(linux::signals(root).built_in_panel);
    assert_eq!(linux::machine(root), Machine::Laptop);
    assert_eq!(encoder_order(&adapters, Machine::Laptop), [intel]);
    // Without the processor's graphics, the sleeping card still isn't woken.
    assert_eq!(auto_encoder(&adapters[1..], Machine::Laptop), None);
    // A desktop has no battery to spare it for.
    assert_eq!(encoder_order(&adapters, Machine::Desktop), [intel, nvidia]);
}

#[test]
fn a_laptops_awake_card_comes_after_its_processors_graphics() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    laptop(root, false);
    let adapters = linux::list(root, false);
    assert_eq!(
        encoder_order(&adapters, Machine::Laptop),
        [&adapters[0], &adapters[1]]
    );
}

#[test]
fn a_desktop_with_one_card_encodes_on_it() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    one_card(root, "nvidia");
    let adapters = linux::list(root, true);
    assert_eq!(adapters.len(), 1, "the framebuffer isn't an adapter");
    let nvidia = &adapters[0];
    assert_eq!(nvidia.key, "pci:0000:01:00.0");
    assert_eq!(nvidia.name, tr!("{} graphics", "NVIDIA"), "without pci.ids");
    assert_eq!(
        nvidia.render_node.as_deref(),
        Some(root.join("dev/dri/renderD128").as_path()),
        "without udev's links"
    );
    assert!(nvidia.drives_display && !nvidia.integrated);
    assert_eq!(linux::machine(root), Machine::Desktop);
    assert_eq!(auto_encoder(&adapters, Machine::Desktop), Some(nvidia));
    assert_eq!(cuda_index(&adapters, nvidia), Some(0));
}

#[test]
fn nouveau_and_a_virtual_machines_adapter_leave_streams_to_the_processor() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    one_card(root, "nouveau");
    let adapters = linux::list(root, false);
    assert_eq!(adapters[0].encoder_api(), None);
    assert_eq!(auto_encoder(&adapters, Machine::Desktop), None);
    assert_eq!(cuda_index(&adapters, &adapters[0]), None);

    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    card(
        root,
        &Card {
            number: 0,
            slot: "0000:00:01.0",
            id: "1AF4:1050",
            driver: "virtio-pci",
            runtime: "active",
            vram: None,
            connectors: &[("Virtual-1", "connected")],
            render: 128,
            by_path: true,
        },
    );
    put(root, "sys/class/dmi/id/chassis_type", "1\n");
    put(root, "usr/share/hwdata/pci.ids", PCI_IDS);
    let adapters = linux::list(root, true);
    assert_eq!(adapters[0].vendor, Vendor::Other);
    assert_eq!(adapters[0].name, "Red Hat, Inc. Virtio 1.0 GPU");
    assert_eq!(linux::machine(root), Machine::Desktop);
    assert_eq!(auto_encoder(&adapters, Machine::Desktop), None);
}

#[test]
fn an_amd_card_with_memory_of_its_own_is_discrete_and_accelerators_arent_listed() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    card(
        root,
        &Card {
            number: 0,
            slot: "0000:03:00.0",
            id: "1002:744C",
            driver: "amdgpu",
            runtime: "active",
            vram: Some(25_753_026_560),
            connectors: &[("DP-1", "connected")],
            render: 128,
            by_path: true,
        },
    );
    // Asleep: only a discrete card sleeps, whatever its memory says.
    card(
        root,
        &Card {
            number: 1,
            slot: "0000:07:00.0",
            id: "1002:73FF",
            driver: "amdgpu",
            runtime: "suspended",
            vram: Some(536_870_912),
            connectors: &[],
            render: 129,
            by_path: true,
        },
    );
    put(
        root,
        "sys/class/drm/card2/device/uevent",
        "DRIVER=amdgpu\nPCI_CLASS=120000\nPCI_ID=1002:74A1\nPCI_SLOT_NAME=0000:0a:00.0\n",
    );
    let adapters = linux::list(root, false);
    let keys: Vec<&str> = adapters
        .iter()
        .map(|adapter| adapter.key.as_str())
        .collect();
    assert_eq!(keys, ["pci:0000:03:00.0", "pci:0000:07:00.0"]);
    assert!(adapters.iter().all(|adapter| !adapter.integrated));
    // The card without a monitor first.
    assert_eq!(
        encoder_order(&adapters, Machine::Desktop),
        [&adapters[1], &adapters[0]]
    );
}

#[test]
fn a_laptop_shows_by_its_panel_its_chassis_or_its_battery() {
    let folder = tempfile::tempdir().expect("a folder");
    let root = folder.path();
    // Firmware that says Unknown, and no battery: the panel tells.
    put(root, "sys/class/dmi/id/chassis_type", "2\n");
    put(root, "sys/class/drm/card0-eDP-1/status", "connected\n");
    assert_eq!(
        linux::signals(root),
        Signals {
            built_in_panel: true,
            chassis: Some(2),
            system_battery: false,
        }
    );
    assert_eq!(linux::machine(root), Machine::Laptop);
    // Some desktops' firmware lists an eDP connector with nothing on it.
    put(root, "sys/class/drm/card0-eDP-1/status", "disconnected\n");
    assert!(!linux::signals(root).built_in_panel);
    assert_eq!(linux::machine(root), Machine::Desktop);
    // A battery whose scope is the system's.
    put(root, "sys/class/power_supply/BAT1/type", "Battery\n");
    put(root, "sys/class/power_supply/BAT1/scope", "System\n");
    assert!(linux::signals(root).system_battery);
    assert_eq!(linux::machine(root), Machine::Laptop);

    let chassis = |kind| {
        classify(
            &Signals {
                chassis: Some(kind),
                ..Signals::default()
            },
            Vec::new,
        )
    };
    for laptop in [8, 9, 10, 14, 30, 31, 32] {
        assert_eq!(chassis(laptop), Machine::Laptop, "chassis type {laptop}");
    }
    for desktop in [3, 4, 6, 7, 13, 35] {
        assert_eq!(chassis(desktop), Machine::Desktop, "chassis type {desktop}");
    }
}

#[test]
fn where_the_firmware_says_nothing_monitors_on_processor_graphics_beside_a_card_mean_a_laptop() {
    let on_integrated = || {
        vec![
            adapter("pci:0000:00:02.0", Vendor::Intel, "i915", true, true),
            adapter("pci:0000:01:00.0", Vendor::Nvidia, "nvidia", false, false),
        ]
    };
    let on_the_card = || {
        vec![
            adapter("pci:0000:00:02.0", Vendor::Intel, "i915", true, false),
            adapter("pci:0000:01:00.0", Vendor::Nvidia, "nvidia", false, true),
        ]
    };
    let unknown = Signals {
        chassis: Some(2),
        ..Signals::default()
    };
    assert_eq!(classify(&unknown, on_integrated), Machine::Laptop);
    assert_eq!(classify(&unknown, on_the_card), Machine::Desktop);
    // No DMI table at all, as in some virtual machines.
    assert_eq!(
        classify(&Signals::default(), on_integrated),
        Machine::Laptop
    );
    // A desktop's chassis outweighs it: a monitor in the motherboard.
    let desktop = Signals {
        chassis: Some(3),
        ..Signals::default()
    };
    assert_eq!(classify(&desktop, on_integrated), Machine::Desktop);
}

#[test]
fn of_two_cards_auto_takes_the_one_without_a_monitor() {
    let adapters = [
        adapter("pci:0000:01:00.0", Vendor::Nvidia, "nvidia", false, true),
        adapter("pci:0000:02:00.0", Vendor::Nvidia, "nvidia", false, false),
    ];
    assert_eq!(
        auto_encoder(&adapters, Machine::Desktop),
        Some(&adapters[1])
    );
    // CUDA counts them in PCI order all the same.
    assert_eq!(cuda_index(&adapters, &adapters[1]), Some(1));
}

#[test]
fn names_are_the_vendors_and_the_model_in_brackets() {
    let name = linux::display_name;
    assert_eq!(
        name(
            Vendor::Nvidia,
            "NVIDIA Corporation",
            "GA102 [GeForce RTX 3080]"
        ),
        "NVIDIA GeForce RTX 3080"
    );
    assert_eq!(
        name(Vendor::Intel, "Intel Corporation", "HD Graphics 620"),
        "Intel HD Graphics 620"
    );
    assert_eq!(
        name(
            Vendor::Amd,
            "Advanced Micro Devices, Inc. [AMD/ATI]",
            "Granite Ridge [AMD Radeon]"
        ),
        "AMD Radeon"
    );
    assert_eq!(
        name(Vendor::Other, "Red Hat, Inc.", "Virtio 1.0 GPU"),
        "Red Hat, Inc. Virtio 1.0 GPU"
    );
}

#[test]
fn the_chassis_type_is_read_from_a_raw_smbios_table() {
    // The BIOS's structure with one string; the System Enclosure, a
    // notebook (10) with a lock (the top bit), without strings; the end.
    let mut table = vec![0u8, 4, 0, 0];
    table.extend(b"Vendor\0\0");
    table.extend([3, 6, 1, 0, 0, 0x80 | 10]);
    table.extend([0, 0]);
    table.extend([127, 4, 2, 0, 0, 0]);
    let mut raw = vec![0u8, 3, 4, 0];
    raw.extend(u32::try_from(table.len()).expect("small").to_le_bytes());
    raw.extend(&table);
    assert_eq!(chassis_from_smbios(&raw), Some(10));
    // Cut before the enclosure, or inside the header: nothing.
    assert_eq!(chassis_from_smbios(&raw[..8 + 12 + 3]), None);
    assert_eq!(chassis_from_smbios(&raw[..6]), None);
    // A structure shorter than its own header.
    let mut broken = raw.clone();
    broken[9] = 2;
    assert_eq!(chassis_from_smbios(&broken), None);
}

/// What the computer running the tests says, to look at by hand, amdgpu's
/// own answer included:
/// `cargo test -p yapfast-core adapters -- --ignored --nocapture`.
#[test]
#[ignore = "reads this computer's /sys and opens its amdgpu render nodes"]
fn this_computer() {
    let adapters = super::list();
    for adapter in &adapters {
        let fused = adapter
            .render_node
            .as_deref()
            .filter(|_| adapter.kernel_driver == "amdgpu" && !adapter.suspended)
            .map(linux::fused);
        println!("{adapter:?}\n  amdgpu says it's in the processor: {fused:?}");
    }
    let machine = super::machine();
    let auto = auto_encoder(&adapters, machine).map(|adapter| adapter.key.as_str());
    println!("{machine:?}, Auto encodes on {auto:?}");
}
