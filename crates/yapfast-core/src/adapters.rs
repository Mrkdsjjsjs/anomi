//! The graphics adapters in this computer, and whether it's a laptop or a
//! desktop, read from what the system already knows: on Linux sysfs, on
//! Windows DXGI and the firmware's tables. Reading loads no driver's library
//! (no Mesa, libva or CUDA), so the window may ask too.
//!
//! Settings lists the adapters under "Encode streams on", which
//! `layout.json` keeps as `stream_encoder`: `"auto"`, `"software"`, or an
//! adapter's [`Adapter::key`]. The stream worker opens its encoder on the
//! adapter chosen, and on [`auto_encoder`]'s when the choice is Auto.
//!
//! Nothing is kept: each [`list`] reads afresh, as whether a card sleeps or
//! drives a monitor changes while Yapfast runs. A list reads a few dozen
//! small sysfs files, asks each awake amdgpu card one `ioctl`, and reads the
//! system's `pci.ids` as far as the last vendor it names. That's for opening
//! Settings and starting a stream, not for every refresh.

// Only Linux and Windows have adapters to list.
#![cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]

use std::path::PathBuf;

use yapfast_lang::tr;

/// A graphics adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Adapter {
    /// What `layout.json`'s `stream_encoder` keeps for it: `pci:` and its PCI
    /// address, `pci:0000:73:00.0`, which holds while the card stays in its
    /// slot, unlike `card0` or `renderD128`. On Windows, without the address,
    /// `luid:` and its LUID, which holds only until a restart.
    pub key: String,
    pub vendor: Vendor,
    /// The PCI device id: `0x13c0` for a Radeon 610M.
    pub device: u16,
    /// Its name for Settings, such as "AMD Radeon 610M": from the system's
    /// PCI id list on Linux and from the driver on Windows, or the vendor's
    /// and "graphics" when neither has one.
    pub name: String,
    /// Built into the processor, sharing main memory.
    pub integrated: bool,
    /// A monitor is plugged into it now.
    pub drives_display: bool,
    /// Where VA-API opens it: `/dev/dri/by-path/pci-<address>-render`, or
    /// `/dev/dri/renderD<n>` where udev made no links. None on Windows.
    pub render_node: Option<PathBuf>,
    /// The kernel's driver, such as `amdgpu`, `i915`, `xe`, `nvidia` or
    /// `nouveau`; libva's driver follows it. Empty on Windows.
    pub kernel_driver: String,
    /// Asleep in runtime power management, as a laptop's discrete card is
    /// while nothing uses it: opening it would wake it. Reading this
    /// doesn't. Always false on Windows.
    pub suspended: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Vendor {
    /// PCI vendor 0x1002.
    Amd,
    /// 0x8086.
    Intel,
    /// 0x10de.
    Nvidia,
    /// Any other: a virtual machine's adapter, a server's management chip.
    Other,
}

/// Whether this computer is a laptop: [`auto_encoder`] doesn't wake a
/// laptop's sleeping card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Machine {
    Desktop,
    Laptop,
}

/// The kind of hardware encoder Yapfast's FFmpeg reaches an adapter's
/// encoder through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncoderApi {
    /// VA-API on the adapter's render node (Linux: amdgpu, i915, xe).
    Vaapi,
    /// NVIDIA's NVENC, through its own driver, not nouveau.
    Nvenc,
    /// AMD's AMF (Windows).
    Amf,
}

impl Vendor {
    fn from_id(id: u16) -> Self {
        match id {
            0x1002 => Self::Amd,
            0x8086 => Self::Intel,
            0x10de => Self::Nvidia,
            _ => Self::Other,
        }
    }

    /// The vendor's name in an adapter's: "AMD Radeon 610M".
    fn brand(self) -> Option<&'static str> {
        match self {
            Self::Amd => Some("AMD"),
            Self::Intel => Some("Intel"),
            Self::Nvidia => Some("NVIDIA"),
            Self::Other => None,
        }
    }
}

impl Adapter {
    /// Its PCI address alone, `0000:73:00.0`; None for a key that isn't one.
    pub fn pci_address(&self) -> Option<&str> {
        self.key.strip_prefix("pci:")
    }

    /// How this build encodes on it, if it can: VA-API for amdgpu, i915 and
    /// xe on Linux, NVENC for NVIDIA's own driver, AMF for AMD on Windows.
    /// Intel on Windows has none: QSV and Media Foundation aren't built.
    pub fn encoder_api(&self) -> Option<EncoderApi> {
        if cfg!(windows) {
            return match self.vendor {
                Vendor::Amd => Some(EncoderApi::Amf),
                Vendor::Nvidia => Some(EncoderApi::Nvenc),
                Vendor::Intel | Vendor::Other => None,
            };
        }
        match (self.vendor, self.kernel_driver.as_str()) {
            (Vendor::Amd, "amdgpu") | (Vendor::Intel, "i915" | "xe") => Some(EncoderApi::Vaapi),
            (Vendor::Nvidia, "nvidia") => Some(EncoderApi::Nvenc),
            _ => None,
        }
    }
}

/// The graphics adapters, in PCI address order, those without an encoder
/// included: a list of choices keeps the ones [`Adapter::encoder_api`] has.
/// Empty where none can be read, as in a sandbox without `/sys`.
pub fn list() -> Vec<Adapter> {
    #[cfg(target_os = "linux")]
    let adapters = linux::list(std::path::Path::new("/"), true);
    #[cfg(windows)]
    let adapters = windows::list();
    #[cfg(not(any(target_os = "linux", windows)))]
    let adapters = Vec::new();
    tracing::debug!(?adapters, "graphics adapters");
    adapters
}

/// Whether this computer is a laptop. In order, a laptop has:
/// 1. a built-in panel (an eDP connector, or LVDS or DSI on older ones);
/// 2. a laptop's chassis type in the firmware's DMI table: Portable (8),
///    Laptop (9), Notebook (10), Sub Notebook (14), Tablet (30),
///    Convertible (31) or Detachable (32);
/// 3. a battery that powers the system, not a wireless mouse's (whose
///    `scope` is `Device`).
///
/// Any other chassis type makes it a desktop. Only where the firmware says
/// Other or Unknown do the adapters decide: monitors on an integrated card
/// beside a discrete one make it a laptop. That alone would be wrong for a
/// desktop whose monitor is plugged into the motherboard, and for a laptop
/// that switches its panel to the discrete card.
pub fn machine() -> Machine {
    #[cfg(target_os = "linux")]
    let machine = linux::machine(std::path::Path::new("/"));
    #[cfg(windows)]
    let machine = classify(&windows::signals(), windows::list);
    #[cfg(not(any(target_os = "linux", windows)))]
    let machine = Machine::Desktop;
    tracing::debug!(?machine, "the kind of computer");
    machine
}

/// The adapter Auto encodes streams on, or None for the processor (x264):
/// the first of [`encoder_order`].
pub fn auto_encoder(adapters: &[Adapter], machine: Machine) -> Option<&Adapter> {
    encoder_order(adapters, machine).first().copied()
}

/// The adapters Auto tries a stream's encoder on, in order, before the
/// processor: the integrated ones, then the discrete ones, a card without a
/// monitor before one with, and none that sleeps on a laptop, which would
/// wake it on battery. Only adapters this build can encode on
/// ([`Adapter::encoder_api`]) are in it.
///
/// Integrated first keeps the card a game draws on free. Today every encoder
/// takes its frames from main memory: the capture reads each one back from
/// the card the compositor draws on, whichever adapter encodes. The
/// integrated adapter then spares the game's card the upload, a CUDA context
/// and its encoder's time. Once a capture hands frames over on the card that
/// composites, the cheapest place to encode will be that card
/// ([`Adapter::drives_display`]), and this is the one function to change.
pub fn encoder_order(adapters: &[Adapter], machine: Machine) -> Vec<&Adapter> {
    let encodes = |adapter: &&Adapter| adapter.encoder_api().is_some();
    let mut order: Vec<&Adapter> = adapters
        .iter()
        .filter(encodes)
        .filter(|adapter| adapter.integrated)
        .collect();
    let mut discrete: Vec<&Adapter> = adapters
        .iter()
        .filter(encodes)
        .filter(|adapter| !adapter.integrated)
        .filter(|adapter| !(machine == Machine::Laptop && adapter.suspended))
        .collect();
    // Stable: those without a monitor first, each kind in PCI order.
    discrete.sort_by_key(|adapter| adapter.drives_display);
    order.append(&mut discrete);
    order
}

/// The adapter `key` names, if it's still there: a card taken out, or an
/// integrated one the firmware turned off, isn't.
pub fn find<'a>(adapters: &'a [Adapter], key: &str) -> Option<&'a Adapter> {
    adapters.iter().find(|adapter| adapter.key == key)
}

/// The number CUDA gives `adapter` with `CUDA_DEVICE_ORDER=PCI_BUS_ID`, which
/// FFmpeg's NVENC takes as its `gpu`: its place among the NVENC adapters in
/// `adapters`, a [`list`], so in PCI order. None for an adapter without
/// NVENC. A card the kernel lists no DRM device for, such as one whose
/// `nvidia-drm` module isn't loaded, would shift the count.
pub fn cuda_index(adapters: &[Adapter], adapter: &Adapter) -> Option<u32> {
    let nvenc = |candidate: &&Adapter| candidate.encoder_api() == Some(EncoderApi::Nvenc);
    if !nvenc(&adapter) {
        return None;
    }
    let place = adapters
        .iter()
        .filter(nvenc)
        .position(|candidate| candidate.key == adapter.key)?;
    u32::try_from(place).ok()
}

/// Video memory of at most this much, 2 GiB, makes an AMD adapter
/// integrated where the driver can't be asked: a processor's carve-out is
/// usually 512 MiB. Wrong for the large carve-outs of Strix Halo.
const SMALL_VRAM: u64 = 2 << 30;

/// What the system says about the kind of computer, for [`machine`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Signals {
    built_in_panel: bool,
    /// SMBIOS's chassis type.
    chassis: Option<u8>,
    system_battery: bool,
}

/// [`machine`]'s rule. `adapters` is read only when the rest says nothing.
fn classify(signals: &Signals, adapters: impl FnOnce() -> Vec<Adapter>) -> Machine {
    const LAPTOP: [u8; 7] = [8, 9, 10, 14, 30, 31, 32];
    if signals.built_in_panel
        || signals.chassis.is_some_and(|kind| LAPTOP.contains(&kind))
        || signals.system_battery
    {
        return Machine::Laptop;
    }
    // 1 is Other and 2 Unknown; 0 isn't a type.
    if signals.chassis.is_some_and(|kind| kind > 2) {
        return Machine::Desktop;
    }
    let adapters = adapters();
    let shown_on_integrated = adapters
        .iter()
        .any(|adapter| adapter.integrated && adapter.drives_display);
    let discrete = adapters
        .iter()
        .any(|adapter| !adapter.integrated && adapter.vendor != Vendor::Other);
    if shown_on_integrated && discrete {
        Machine::Laptop
    } else {
        Machine::Desktop
    }
}

/// A name for an adapter the system names nothing: "AMD graphics", or its
/// PCI ids for another vendor's.
fn fallback_name(vendor: Vendor, vendor_id: u16, device: u16) -> String {
    match vendor.brand() {
        Some(brand) => tr!("{} graphics", brand),
        None => tr!("{} graphics", format!("{vendor_id:04x}:{device:04x}")),
    }
}

/// The chassis type in the firmware's SMBIOS table as Windows hands it
/// over: eight bytes of header, the last four the table's length, then the
/// table's structures. The System Enclosure's, type 3, holds it at byte 5.
#[cfg(any(windows, test))]
fn chassis_from_smbios(raw: &[u8]) -> Option<u8> {
    let length = u32::from_le_bytes(raw.get(4..8)?.try_into().ok()?);
    let mut table = raw.get(8..)?;
    table = table.get(..table.len().min(usize::try_from(length).ok()?))?;
    while table.len() >= 4 {
        let (kind, formatted) = (table[0], usize::from(table[1]));
        if formatted < 4 || formatted > table.len() {
            return None;
        }
        if kind == 3 {
            // The top bit says the chassis has a lock.
            return table[..formatted].get(5).map(|kind| kind & 0x7f);
        }
        if kind == 127 {
            return None;
        }
        // Each structure's strings follow it and end with two zero bytes.
        let strings = &table[formatted..];
        let end = strings.windows(2).position(|pair| pair == [0, 0])?;
        table = &strings[end + 2..];
    }
    None
}

#[cfg(target_os = "linux")]
mod linux {
    //! Adapters from `/sys/class/drm`, where the kernel lists each card it
    //! drives and its connectors. `root` is `/` but in tests, which read a
    //! copy of a computer's tree.

    use std::{
        fs,
        io::{BufRead as _, BufReader},
        path::{Path, PathBuf},
    };

    use super::{Adapter, Machine, SMALL_VRAM, Signals, Vendor};

    pub(super) fn machine(root: &Path) -> Machine {
        super::classify(&signals(root), || list(root, false))
    }

    /// Each display adapter a kernel driver drives, in PCI address order;
    /// `named` looks their names up in `pci.ids`.
    pub(super) fn list(root: &Path, named: bool) -> Vec<Adapter> {
        let drm = root.join("sys/class/drm");
        let (cards, connectors) = entries(&drm);
        let mut found: Vec<(Adapter, u16)> = cards
            .iter()
            .filter_map(|card| read_card(root, &drm, card, &connectors))
            .collect();
        found.sort_by(|(a, _), (b, _)| a.key.cmp(&b.key));
        found.dedup_by(|(a, _), (b, _)| a.key == b.key);
        if named {
            let ids: Vec<(u16, u16)> = found
                .iter()
                .map(|(adapter, vendor_id)| (*vendor_id, adapter.device))
                .collect();
            for ((adapter, vendor_id), name) in found.iter_mut().zip(names(root, &ids)) {
                adapter.name = name.unwrap_or_else(|| {
                    super::fallback_name(adapter.vendor, *vendor_id, adapter.device)
                });
            }
        }
        found.into_iter().map(|(adapter, _)| adapter).collect()
    }

    /// What the system says about the kind of computer.
    pub(super) fn signals(root: &Path) -> Signals {
        let drm = root.join("sys/class/drm");
        let (_, connectors) = entries(&drm);
        // A panel with its lid shut stays connected; some desktops' firmware
        // lists an eDP connector with nothing on it.
        let built_in_panel = connectors
            .iter()
            .filter(|name| built_in(name))
            .any(|name| read(&drm.join(name).join("status")).as_deref() != Some("disconnected"));
        let chassis =
            read(&root.join("sys/class/dmi/id/chassis_type")).and_then(|kind| kind.parse().ok());
        Signals {
            built_in_panel,
            chassis,
            system_battery: system_battery(root),
        }
    }

    /// The names in `/sys/class/drm` of cards, `card1`, and of their
    /// connectors, `card1-DP-3`. Render nodes and `version` aren't either.
    fn entries(drm: &Path) -> (Vec<String>, Vec<String>) {
        let (mut cards, mut connectors) = (Vec::new(), Vec::new());
        let Ok(listing) = fs::read_dir(drm) else {
            return (cards, connectors);
        };
        for entry in listing.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let Some(rest) = name.strip_prefix("card") else {
                continue;
            };
            let number = rest.split_once('-').map_or(rest, |(number, _)| number);
            if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            if number.len() == rest.len() {
                cards.push(name);
            } else {
                connectors.push(name);
            }
        }
        (cards, connectors)
    }

    /// The card `card` (`card0`), with its PCI vendor id, unnamed; None for
    /// one that isn't a PCI display adapter, such as the firmware's
    /// framebuffer early in boot.
    fn read_card(
        root: &Path,
        drm: &Path,
        card: &str,
        connectors: &[String],
    ) -> Option<(Adapter, u16)> {
        let device = drm.join(card).join("device");
        let uevent = fs::read_to_string(device.join("uevent")).ok()?;
        let slot = field(&uevent, "PCI_SLOT_NAME")?.to_ascii_lowercase();
        // Class 03, a display controller: not an accelerator a graphics
        // driver also drives.
        let class =
            field(&uevent, "PCI_CLASS").and_then(|class| u32::from_str_radix(class, 16).ok());
        if class.is_some_and(|class| class >> 16 != 0x03) {
            return None;
        }
        let (vendor_id, device_id) = field(&uevent, "PCI_ID")?.split_once(':')?;
        let vendor_id = u16::from_str_radix(vendor_id, 16).ok()?;
        let device_id = u16::from_str_radix(device_id, 16).ok()?;
        let vendor = Vendor::from_id(vendor_id);
        let kernel_driver = field(&uevent, "DRIVER").unwrap_or_default().to_owned();
        let suspended = read(&device.join("power/runtime_status")).as_deref() == Some("suspended");
        // The kernel keeps a connector's status from its last probe; reading
        // it doesn't probe again, nor wake the card.
        let prefix = format!("{card}-");
        let drives_display = connectors
            .iter()
            .filter(|connector| connector.starts_with(&prefix))
            .any(|connector| {
                read(&drm.join(connector).join("status")).as_deref() == Some("connected")
            });
        let render_node = render_node(root, &device, &slot);
        let integrated = match vendor {
            // Intel's processors put their graphics at 00:02.0, its Arc cards elsewhere.
            Vendor::Intel => slot.ends_with(":00:02.0"),
            // amdgpu puts only discrete cards to sleep.
            Vendor::Amd if suspended => false,
            Vendor::Amd if kernel_driver == "amdgpu" => render_node
                .as_deref()
                .and_then(fused)
                .unwrap_or_else(|| small_vram(&device)),
            // The older radeon driver's processors have their graphics on bus 0.
            Vendor::Amd => slot.get(5..7) == Some("00"),
            Vendor::Nvidia | Vendor::Other => false,
        };
        let adapter = Adapter {
            key: format!("pci:{slot}"),
            vendor,
            device: device_id,
            name: String::new(),
            integrated,
            drives_display,
            render_node,
            kernel_driver,
            suspended,
        };
        Some((adapter, vendor_id))
    }

    /// The value of `key` in a uevent file's `KEY=value` lines.
    fn field<'a>(uevent: &'a str, key: &str) -> Option<&'a str> {
        uevent
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    }

    /// A sysfs file's one value.
    fn read(path: &Path) -> Option<String> {
        let mut text = fs::read_to_string(path).ok()?;
        text.truncate(text.trim_end().len());
        Some(text)
    }

    /// The card's render node: udev's link by PCI address, or the node the
    /// card's sysfs entry names.
    fn render_node(root: &Path, device: &Path, slot: &str) -> Option<PathBuf> {
        let by_path = root.join(format!("dev/dri/by-path/pci-{slot}-render"));
        if by_path.exists() {
            return Some(by_path);
        }
        fs::read_dir(device.join("drm"))
            .ok()?
            .flatten()
            .find_map(|entry| {
                let name = entry.file_name().into_string().ok()?;
                name.starts_with("renderD")
                    .then(|| root.join("dev/dri").join(name))
            })
            .filter(|node| node.exists())
    }

    /// Whether amdgpu calls the card part of the processor, an APU: its
    /// `AMDGPU_INFO_DEV_INFO` answer has `AMDGPU_IDS_FLAGS_FUSION`. Asked
    /// through the render node, which loads no library; the card is awake
    /// already. None where it can't be asked, such as in a sandbox without
    /// the node.
    pub(super) fn fused(render_node: &Path) -> Option<bool> {
        use std::os::fd::AsRawFd as _;

        /// `struct drm_amdgpu_info` from the kernel's `amdgpu_drm.h`: where
        /// the answer goes, its size, the query, and a union of 16 bytes
        /// this query doesn't use.
        #[repr(C)]
        #[allow(dead_code)]
        struct Info {
            return_pointer: u64,
            return_size: u32,
            query: u32,
            unused: [u32; 4],
        }
        /// `DRM_IOW(DRM_COMMAND_BASE + DRM_AMDGPU_INFO, struct drm_amdgpu_info)`.
        const DRM_IOCTL_AMDGPU_INFO: u32 = 0x4020_6445;
        const AMDGPU_INFO_DEV_INFO: u32 = 0x16;
        const AMDGPU_IDS_FLAGS_FUSION: u64 = 0x1;
        /// Where `ids_flags` is in `struct drm_amdgpu_info_device`, counted
        /// in u64s: at byte 136. The kernel writes no more than it's given.
        const IDS_FLAGS: usize = 17;

        let node = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(render_node)
            .ok()?;
        let mut answer = [0u64; IDS_FLAGS + 1];
        let info = Info {
            return_pointer: answer.as_mut_ptr() as u64,
            return_size: (IDS_FLAGS as u32 + 1) * 8,
            query: AMDGPU_INFO_DEV_INFO,
            unused: [0; 4],
        };
        // SAFETY: `info` is the request this ioctl reads, and points the
        // kernel at `answer`, which outlives the call, for at most
        // `return_size` bytes. Anything but a render node of amdgpu's
        // answers with an error, such as ENOTTY for a plain file.
        let asked = unsafe {
            libc::ioctl(
                node.as_raw_fd(),
                DRM_IOCTL_AMDGPU_INFO as _,
                &raw const info,
            )
        };
        if asked != 0 {
            return None;
        }
        Some(answer[IDS_FLAGS] & AMDGPU_IDS_FLAGS_FUSION != 0)
    }

    /// Whether the card has a processor's small carve-out of memory, where
    /// amdgpu can't be asked.
    fn small_vram(device: &Path) -> bool {
        read(&device.join("mem_info_vram_total"))
            .and_then(|bytes| bytes.parse::<u64>().ok())
            .is_some_and(|bytes| bytes <= SMALL_VRAM)
    }

    /// A built-in panel's connector: `card0-eDP-1`, an older laptop's LVDS,
    /// or a tablet's DSI.
    fn built_in(connector: &str) -> bool {
        connector.split_once('-').is_some_and(|(_, kind)| {
            ["eDP-", "LVDS-", "DSI-"]
                .iter()
                .any(|panel| kind.starts_with(panel))
        })
    }

    /// A battery that powers the computer. A wireless mouse's or a
    /// controller's says its scope is `Device`; a laptop's says `System` or
    /// nothing.
    fn system_battery(root: &Path) -> bool {
        let Ok(supplies) = fs::read_dir(root.join("sys/class/power_supply")) else {
            return false;
        };
        supplies.flatten().any(|supply| {
            let supply = supply.path();
            read(&supply.join("type")).as_deref() == Some("Battery")
                && read(&supply.join("scope")).as_deref() != Some("Device")
        })
    }

    /// Where distributions put the PCI id list.
    const PCI_IDS: [&str; 3] = [
        "usr/share/hwdata/pci.ids",
        "usr/share/misc/pci.ids",
        "usr/share/pci.ids",
    ];

    /// Each `(vendor, device)`'s name for Settings, from `pci.ids`: one pass,
    /// which stops after the last vendor wanted, the list being sorted by
    /// vendor.
    fn names(root: &Path, ids: &[(u16, u16)]) -> Vec<Option<String>> {
        let mut names = vec![None; ids.len()];
        let Some(last) = ids.iter().map(|(vendor, _)| *vendor).max() else {
            return names;
        };
        let Some(file) = PCI_IDS
            .iter()
            .find_map(|path| fs::File::open(root.join(path)).ok())
        else {
            return names;
        };
        let mut reader = BufReader::new(file);
        let mut bytes = Vec::new();
        // The vendor whose devices follow, if one wanted, and its name.
        let mut vendor: Option<(u16, String)> = None;
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&bytes);
            let line = line.trim_end();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(device_line) = line.strip_prefix('\t') {
                // Two tabs: a subsystem, a board maker's own name.
                if device_line.starts_with('\t') {
                    continue;
                }
                let Some((vendor_id, vendor_name)) = &vendor else {
                    continue;
                };
                let Some((device, name)) = entry(device_line) else {
                    continue;
                };
                for (slot, _) in names
                    .iter_mut()
                    .zip(ids)
                    .filter(|(_, wanted)| **wanted == (*vendor_id, device))
                {
                    *slot = Some(display_name(Vendor::from_id(*vendor_id), vendor_name, name));
                }
                continue;
            }
            // A vendor's line; after the last, the device classes, "C 00".
            let Some((id, name)) = entry(line) else {
                break;
            };
            if id > last {
                break;
            }
            vendor = ids
                .iter()
                .any(|(wanted, _)| *wanted == id)
                .then(|| (id, name.to_owned()));
        }
        names
    }

    /// A `pci.ids` line's id and name: `13c0  Granite Ridge [Radeon 610M]`.
    fn entry(line: &str) -> Option<(u16, &str)> {
        let (id, name) = line.split_once("  ")?;
        Some((u16::from_str_radix(id, 16).ok()?, name.trim()))
    }

    /// The name Settings shows: the vendor's, then the model, which is the
    /// part of `pci.ids`' name in brackets where it has one. "GA102
    /// [GeForce RTX 3080]" is "NVIDIA GeForce RTX 3080".
    pub(super) fn display_name(vendor: Vendor, vendor_name: &str, device_name: &str) -> String {
        let model = match (device_name.find('['), device_name.rfind(']')) {
            (Some(open), Some(close)) if open + 1 < close => device_name[open + 1..close].trim(),
            _ => device_name.trim(),
        };
        let brand = vendor.brand().unwrap_or(vendor_name);
        if brand.is_empty() || model.starts_with(brand) {
            model.to_owned()
        } else {
            format!("{brand} {model}")
        }
    }
}

#[cfg(windows)]
mod windows {
    //! Adapters from DXGI, their PCI addresses from D3DKMT, and the kind of
    //! computer from its battery and the firmware's SMBIOS table, through
    //! the system's own DLLs. None of these load a graphics driver's
    //! user-mode library.
    //!
    //! UNTESTED: written to compile; this computer runs Linux. Check on
    //! Windows that the keys read `pci:0000:<bus>:<device>.<function>` and
    //! that a laptop's integrated card says it drives the panel.

    use std::ffi::c_void;

    use super::{Adapter, SMALL_VRAM, Signals, Vendor};

    #[repr(C)]
    #[allow(dead_code)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    /// `IID_IDXGIFactory1`.
    const FACTORY1: Guid = Guid {
        data1: 0x770a_ae78,
        data2: 0xf26f,
        data3: 0x4dba,
        data4: [0xa8, 0x29, 0x25, 0x3c, 0x83, 0xd1, 0xb3, 0x87],
    };

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Luid {
        low: u32,
        high: i32,
    }

    /// `DXGI_ADAPTER_DESC1`.
    #[repr(C)]
    #[allow(dead_code)]
    struct Desc {
        description: [u16; 128],
        vendor_id: u32,
        device_id: u32,
        subsystem: u32,
        revision: u32,
        dedicated_video_memory: usize,
        dedicated_system_memory: usize,
        shared_system_memory: usize,
        luid: Luid,
        flags: u32,
    }

    /// `DXGI_ADAPTER_FLAG_SOFTWARE`: Microsoft's Basic Render Driver.
    const SOFTWARE: u32 = 2;

    // Places in the COM tables: IUnknown's three functions come first, then
    // IDXGIObject's four, then each interface's own.
    const RELEASE: usize = 2;
    /// After IDXGIFactory's EnumAdapters, MakeWindowAssociation,
    /// GetWindowAssociation, CreateSwapChain and CreateSoftwareAdapter.
    const ENUM_ADAPTERS1: usize = 12;
    const ENUM_OUTPUTS: usize = 7;
    /// After IDXGIAdapter's EnumOutputs, GetDesc and CheckInterfaceSupport.
    const GET_DESC1: usize = 10;

    type Release = unsafe extern "system" fn(*mut c_void) -> u32;
    type Enumerate = unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> i32;
    type GetDesc1 = unsafe extern "system" fn(*mut c_void, *mut Desc) -> i32;

    /// `D3DKMT_OPENADAPTERFROMLUID`.
    #[repr(C)]
    #[allow(dead_code)]
    struct OpenAdapter {
        luid: Luid,
        adapter: u32,
    }

    /// `D3DKMT_QUERYADAPTERINFO`.
    #[repr(C)]
    #[allow(dead_code)]
    struct QueryAdapterInfo {
        adapter: u32,
        kind: u32,
        data: *mut c_void,
        size: u32,
    }

    /// `D3DKMT_CLOSEADAPTER`.
    #[repr(C)]
    #[allow(dead_code)]
    struct CloseAdapter {
        adapter: u32,
    }

    /// `D3DKMT_ADAPTERADDRESS`.
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct PciAddress {
        bus: u32,
        device: u32,
        function: u32,
    }

    /// `KMTQAITYPE_ADAPTERADDRESS`.
    const ADAPTER_ADDRESS: u32 = 6;

    /// `SYSTEM_POWER_STATUS`.
    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)]
    struct PowerStatus {
        ac_line: u8,
        battery_flag: u8,
        battery_percent: u8,
        system_status: u8,
        battery_life: u32,
        battery_full_life: u32,
    }

    /// `BatteryFlag`'s "no system battery"; 255, unknown, has it too.
    const NO_BATTERY: u8 = 128;

    #[allow(non_snake_case)]
    #[link(name = "dxgi", kind = "raw-dylib")]
    unsafe extern "system" {
        fn CreateDXGIFactory1(riid: *const Guid, factory: *mut *mut c_void) -> i32;
    }

    #[allow(non_snake_case)]
    #[link(name = "gdi32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn D3DKMTOpenAdapterFromLuid(open: *mut OpenAdapter) -> i32;
        fn D3DKMTQueryAdapterInfo(query: *const QueryAdapterInfo) -> i32;
        fn D3DKMTCloseAdapter(close: *const CloseAdapter) -> i32;
    }

    #[allow(non_snake_case)]
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetSystemPowerStatus(status: *mut PowerStatus) -> i32;
        fn GetSystemFirmwareTable(provider: u32, id: u32, buffer: *mut c_void, size: u32) -> u32;
    }

    /// The function at `index` in a COM object's table.
    ///
    /// # Safety
    /// `object` is a live COM object whose interface has a function there.
    unsafe fn method(object: *mut c_void, index: usize) -> *const c_void {
        // SAFETY: a COM object starts with a pointer to its table.
        unsafe { *(*(object as *const *const *const c_void)).add(index) }
    }

    /// # Safety
    /// `object` is a live COM object, and this drops the reference held.
    unsafe fn release(object: *mut c_void) {
        // SAFETY: every COM table has IUnknown's Release third.
        unsafe {
            let release = std::mem::transmute::<*const c_void, Release>(method(object, RELEASE));
            release(object);
        }
    }

    pub(super) fn list() -> Vec<Adapter> {
        let mut factory: *mut c_void = std::ptr::null_mut();
        // SAFETY: FACTORY1 names the interface `factory` receives.
        if unsafe { CreateDXGIFactory1(&FACTORY1, &mut factory) } < 0 || factory.is_null() {
            return Vec::new();
        }
        // SAFETY: `factory` is an IDXGIFactory1, which has EnumAdapters1 there.
        let enumerate = unsafe {
            std::mem::transmute::<*const c_void, Enumerate>(method(factory, ENUM_ADAPTERS1))
        };
        let mut adapters = Vec::new();
        for index in 0u32.. {
            let mut adapter: *mut c_void = std::ptr::null_mut();
            // SAFETY: as above; past the last, DXGI_ERROR_NOT_FOUND is negative.
            if unsafe { enumerate(factory, index, &mut adapter) } < 0 || adapter.is_null() {
                break;
            }
            // SAFETY: EnumAdapters1 handed over an IDXGIAdapter1, released after.
            if let Some(found) = unsafe { describe(adapter) } {
                adapters.push(found);
            }
            // SAFETY: the reference EnumAdapters1 handed over.
            unsafe { release(adapter) };
        }
        // SAFETY: the reference CreateDXGIFactory1 handed over.
        unsafe { release(factory) };
        adapters.sort_by(|a, b| a.key.cmp(&b.key));
        adapters.dedup_by(|a, b| a.key == b.key);
        adapters
    }

    /// # Safety
    /// `adapter` is a live IDXGIAdapter1.
    unsafe fn describe(adapter: *mut c_void) -> Option<Adapter> {
        // SAFETY: zeroes are a DXGI_ADAPTER_DESC1, which GetDesc1 fills.
        let mut desc: Desc = unsafe { std::mem::zeroed() };
        // SAFETY: an IDXGIAdapter1 has GetDesc1 there.
        let get_desc =
            unsafe { std::mem::transmute::<*const c_void, GetDesc1>(method(adapter, GET_DESC1)) };
        // SAFETY: `desc` is the struct it fills.
        if unsafe { get_desc(adapter, &mut desc) } < 0 || desc.flags & SOFTWARE != 0 {
            return None;
        }
        let vendor_id = u16::try_from(desc.vendor_id).ok()?;
        let device = u16::try_from(desc.device_id).ok()?;
        let vendor = Vendor::from_id(vendor_id);
        let address = pci_address(desc.luid);
        let key = match address {
            // D3DKMT gives no PCI domain; a desktop's is 0.
            Some(PciAddress {
                bus,
                device: slot,
                function,
            }) => format!("pci:0000:{bus:02x}:{slot:02x}.{function:x}"),
            None => format!("luid:{:08x}{:08x}", desc.luid.high, desc.luid.low),
        };
        // The outputs DXGI lists are those a monitor shows the desktop on.
        let mut output: *mut c_void = std::ptr::null_mut();
        // SAFETY: an IDXGIAdapter1 has EnumOutputs there; an output it hands
        // over is released below.
        let drives_display = unsafe {
            let enumerate =
                std::mem::transmute::<*const c_void, Enumerate>(method(adapter, ENUM_OUTPUTS));
            enumerate(adapter, 0, &mut output) >= 0 && !output.is_null()
        };
        if drives_display {
            // SAFETY: the reference EnumOutputs handed over.
            unsafe { release(output) };
        }
        let dedicated = desc.dedicated_video_memory as u64;
        let integrated = match vendor {
            Vendor::Intel => address.map_or(dedicated <= SMALL_VRAM, |at| {
                at.bus == 0 && at.device == 2 && at.function == 0
            }),
            Vendor::Amd => dedicated <= SMALL_VRAM,
            Vendor::Nvidia | Vendor::Other => false,
        };
        let length = desc
            .description
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(desc.description.len());
        let name = String::from_utf16_lossy(&desc.description[..length]);
        let name = match name.trim() {
            "" => super::fallback_name(vendor, vendor_id, device),
            name => name.to_owned(),
        };
        Some(Adapter {
            key,
            vendor,
            device,
            name,
            integrated,
            drives_display,
            render_node: None,
            kernel_driver: String::new(),
            suspended: false,
        })
    }

    /// The adapter's PCI bus, device and function, from the kernel's
    /// graphics interface.
    fn pci_address(luid: Luid) -> Option<PciAddress> {
        let mut open = OpenAdapter { luid, adapter: 0 };
        // SAFETY: `open` is the struct the call reads and fills.
        if unsafe { D3DKMTOpenAdapterFromLuid(&mut open) } != 0 {
            return None;
        }
        let mut address = PciAddress::default();
        let query = QueryAdapterInfo {
            adapter: open.adapter,
            kind: ADAPTER_ADDRESS,
            data: (&raw mut address).cast(),
            size: std::mem::size_of::<PciAddress>() as u32,
        };
        // SAFETY: `query` points at `address`, of the size it says.
        let asked = unsafe { D3DKMTQueryAdapterInfo(&query) };
        // SAFETY: the handle opened above, closed once.
        unsafe {
            D3DKMTCloseAdapter(&CloseAdapter {
                adapter: open.adapter,
            })
        };
        (asked == 0).then_some(address)
    }

    /// What the system says about the kind of computer. These calls don't
    /// say which output is a built-in panel.
    pub(super) fn signals() -> Signals {
        Signals {
            built_in_panel: false,
            chassis: chassis(),
            system_battery: system_battery(),
        }
    }

    fn system_battery() -> bool {
        let mut status = PowerStatus::default();
        // SAFETY: `status` is the SYSTEM_POWER_STATUS the call fills.
        if unsafe { GetSystemPowerStatus(&mut status) } == 0 {
            return false;
        }
        status.battery_flag & NO_BATTERY == 0
    }

    /// The chassis type in the firmware's SMBIOS table.
    fn chassis() -> Option<u8> {
        /// The raw SMBIOS table's provider, 'RSMB'.
        const RSMB: u32 = u32::from_be_bytes(*b"RSMB");
        // SAFETY: with no buffer, the call says the size it needs.
        let size = unsafe { GetSystemFirmwareTable(RSMB, 0, std::ptr::null_mut(), 0) };
        if size == 0 {
            return None;
        }
        let mut raw = vec![0u8; usize::try_from(size).ok()?];
        // SAFETY: `raw` holds the `size` bytes the call may write.
        let read = unsafe { GetSystemFirmwareTable(RSMB, 0, raw.as_mut_ptr().cast(), size) };
        if read == 0 || read > size {
            return None;
        }
        raw.truncate(usize::try_from(read).ok()?);
        super::chassis_from_smbios(&raw)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
