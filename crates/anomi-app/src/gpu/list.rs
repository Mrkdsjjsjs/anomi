//! The adapters Settings offers. Which ones Vulkan can draw with is known
//! only by enumerating them, which loads every installed Vulkan driver into
//! the process enumerating, for good: so a child does it, `anomi
//! --list-adapters`, once a run, the first time Settings asks, and prints
//! them as JSON, the system's adapters (`yapfast_core::adapters`) matched
//! with Vulkan's by PCI address. Until it answers, Settings shows the
//! system's list, each adapter usable where its Vulkan driver is installed,
//! and builds the page again when it does. An adapter the window's own
//! start found unable to present to it is never usable.

use std::io::Read as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use iced_wgpu::adapter::{Info, Kind};
use yapfast_core::adapters::{Adapter, Vendor};

use super::{Due, Row, drivers};

/// The argument the child is started with.
pub const ARGUMENT: &str = "--list-adapters";

/// The child's list, once it answered (or failed: the system's then).
static LISTED: OnceLock<Arc<[Row]>> = OnceLock::new();
static ASKED: AtomicBool = AtomicBool::new(false);

/// How long the child may take, and how often it's looked at meanwhile.
const PATIENCE: Duration = Duration::from_secs(10);
const LOOK: Duration = Duration::from_millis(25);

/// `anomi --list-adapters`: the adapters, as JSON on standard output. Run
/// in a process of its own, which ends after.
pub fn print() -> anyhow::Result<()> {
    let rows = merge(&yapfast_core::adapters::list(), &iced_wgpu::adapter::list());
    println!("{}", serde_json::to_string(&rows)?);
    Ok(())
}

/// The adapters as known now; the first call in the program asks the
/// child.
pub(super) fn rows() -> Arc<[Row]> {
    let rows = match LISTED.get() {
        Some(rows) => Arc::clone(rows),
        None => {
            if super::live() && !ASKED.swap(true, Ordering::AcqRel) {
                ask();
            }
            system()
        }
    };
    presented(rows)
}

/// Starts the child on a thread of its own, which tells `update` once it
/// answered.
fn ask() {
    let asked = std::thread::Builder::new()
        .name("anomi-adapters".into())
        .spawn(|| {
            let rows: Arc<[Row]> = match child() {
                Some(rows) => Arc::from(rows),
                None => system(),
            };
            let _ = LISTED.set(rows);
            super::tell(Due::Listed);
        });
    if let Err(error) = asked {
        tracing::warn!(%error, "could not list the graphics adapters");
    }
}

/// Runs `anomi --list-adapters` and reads its answer.
fn child() -> Option<Vec<Row>> {
    let program = if cfg!(target_os = "linux") {
        PathBuf::from("/proc/self/exe")
    } else {
        std::env::current_exe().ok()?
    };
    let mut command = Command::new(program);
    command
        .arg(ARGUMENT)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // The child sees every driver; this process let load only the one
    // that draws.
    if drivers::ours() {
        command.env_remove(drivers::SELECT);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(%error, "could not start listing the graphics adapters");
            return None;
        }
    };
    let begun = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(status)) => {
                tracing::warn!(%status, "listing the graphics adapters failed");
                return None;
            }
            Ok(None) if begun.elapsed() < PATIENCE => std::thread::sleep(LOOK),
            Ok(None) => {
                tracing::warn!("listing the graphics adapters took too long");
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Err(error) => {
                tracing::warn!(%error, "could not wait for the graphics adapters' list");
                return None;
            }
        }
    }
    let mut answer = String::new();
    child.stdout.take()?.read_to_string(&mut answer).ok()?;
    // Its last line: a driver may have written to standard output before.
    let answer = answer.lines().rev().find(|line| line.starts_with('['))?;
    match serde_json::from_str(answer) {
        Ok(rows) => Some(rows),
        Err(error) => {
            tracing::warn!(%error, "the graphics adapters' list didn't read");
            None
        }
    }
}

/// The system's adapters, each usable where its Vulkan driver is
/// installed.
fn system() -> Arc<[Row]> {
    let cards = yapfast_core::adapters::list();
    cards
        .iter()
        .map(|card| row(card, drivers::installed(card)))
        .collect()
}

fn row(card: &Adapter, usable: bool) -> Row {
    Row {
        id: card.key.clone(),
        name: card.name.clone(),
        integrated: card.integrated,
        drives_display: card.drives_display,
        usable,
    }
}

/// The system's adapters, each usable where Vulkan offers it, then those
/// Vulkan offers that the system doesn't list, by their PCI address.
pub(super) fn merge(cards: &[Adapter], vulkan: &[Info]) -> Vec<Row> {
    let mut rows: Vec<Row> = cards
        .iter()
        .map(|card| row(card, vulkan.iter().any(|info| same(card, info))))
        .collect();
    for info in vulkan {
        if info.bus.is_empty() || cards.iter().any(|card| same(card, info)) {
            continue;
        }
        rows.push(Row {
            id: format!("pci:{}", info.bus.to_ascii_lowercase()),
            name: info.name.clone(),
            integrated: info.kind == Kind::Integrated,
            drives_display: false,
            usable: true,
        });
    }
    rows
}

/// Whether Vulkan's adapter is the system's: the same PCI address, or
/// where either doesn't say one, the same vendor and device ids.
pub(super) fn same(card: &Adapter, info: &Info) -> bool {
    match card.pci_address() {
        Some(address) if !info.bus.is_empty() => address.eq_ignore_ascii_case(&info.bus),
        _ => vendor_id(card.vendor) == Some(info.vendor) && u32::from(card.device) == info.device,
    }
}

/// A vendor's PCI id.
pub(super) fn vendor_id(vendor: Vendor) -> Option<u32> {
    match vendor {
        Vendor::Amd => Some(0x1002),
        Vendor::Intel => Some(0x8086),
        Vendor::Nvidia => Some(0x10de),
        Vendor::Other => None,
    }
}

/// `rows`, with an adapter the window's start found unable to present to
/// it not usable.
fn presented(rows: Arc<[Row]>) -> Arc<[Row]> {
    let seen = iced_wgpu::adapter::seen();
    let refused = |row: &Row| {
        seen.iter().any(|info| {
            info.presents == Some(false)
                && (row.id.strip_prefix("pci:"))
                    .is_some_and(|bus| bus.eq_ignore_ascii_case(&info.bus))
        })
    };
    if !rows.iter().any(|row| row.usable && refused(row)) {
        return rows;
    }
    rows.iter()
        .map(|row| Row {
            usable: row.usable && !refused(row),
            ..row.clone()
        })
        .collect()
}
