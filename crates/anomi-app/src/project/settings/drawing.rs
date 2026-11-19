//! Advanced's "Interface rendering", as the page offers it: Automatic, the
//! processor and each graphics adapter [`gpu::adapters`] lists, the one
//! `layout.json` keeps as `interface_renderer`, and what draws the interface
//! now ([`gpu::running`]). The adapters were listed once, in a child
//! process, so building the page loads no graphics driver into this one.
//! An adapter Vulkan can't draw with says why; on Linux that's a missing
//! Vulkan driver, named by the family that has it, read from the system's
//! own list of adapters (`yapfast_core::adapters`), as "Encode streams on"
//! reads it.

use anomi_vm::settings::advanced::DrawOn;
use yapfast_core::adapters::{self, Adapter, Vendor};
use yapfast_lang::tr;

use crate::gpu::{self, Choice, Row, Running};
use crate::layout::Layout;

/// `interface_renderer`'s keys for Automatic and the processor, as
/// [`Choice::parse`] reads them; an adapter's is its id.
const AUTO: &str = "auto";
const PROCESSOR: &str = "processor";

/// "Interface rendering" as the page shows it: its rows, the one chosen,
/// and the line under them.
pub(super) struct Drawing {
    pub rows: Vec<DrawOn>,
    pub chosen: usize,
    pub line: String,
}

/// The rows for `layout`'s choice; `cards` is the system's list of
/// adapters, read once for the page.
pub(super) fn drawing(layout: &Layout, cards: &[Adapter]) -> Drawing {
    let found = gpu::adapters();
    let running = gpu::running();
    build(
        layout.interface_renderer.as_deref(),
        &found,
        cards,
        &running,
    )
}

/// [`drawing`] from what it reads.
fn build(saved: Option<&str>, found: &[Row], cards: &[Adapter], running: &Running) -> Drawing {
    let processor = tr!(
        "The graphics cards stay free and the least memory is used, but a large window redraws slower."
    );
    let mut rows = vec![
        DrawOn {
            title: tr!("Automatic").to_owned(),
            description: automatic(found),
            key: AUTO.to_owned(),
            usable: true,
        },
        DrawOn {
            title: tr!("Processor").to_owned(),
            description: processor.to_owned(),
            key: PROCESSOR.to_owned(),
            usable: true,
        },
    ];
    rows.extend(found.iter().map(|row| adapter(row, cards)));
    let chosen = match Choice::parse(saved) {
        Choice::Auto => 0,
        Choice::Processor => 1,
        Choice::Adapter(id) => match rows.iter().position(|row| row.key == id) {
            Some(at) => at,
            // Kept chosen, so the page says what was asked for.
            None => {
                rows.push(gone(&id));
                rows.len() - 1
            }
        },
    };
    Drawing {
        rows,
        chosen,
        line: now(running, found),
    }
}

/// What Automatic takes on this computer: the integrated graphics, else the
/// processor.
fn automatic(found: &[Row]) -> String {
    let draws = found.iter().find(|row| row.integrated && row.usable);
    let built_in = found.iter().find(|row| row.integrated);
    match (draws, built_in) {
        (Some(row), _) => tr!(
            "The integrated graphics, else the processor. Here that's {}.",
            row.name
        ),
        (None, Some(row)) => tr!(
            "The integrated graphics, else the processor. Here that's the processor, until {} can draw.",
            row.name
        ),
        (None, None) => tr!(
            "The integrated graphics, else the processor. This computer has none, so it's the processor."
        )
        .to_owned(),
    }
}

/// An adapter's row: integrated or discrete (the card games run on), the
/// monitor on it, and why it can't draw where it can't.
fn adapter(row: &Row, cards: &[Adapter]) -> DrawOn {
    let kind = match (row.integrated, row.drives_display) {
        (true, false) => tr!("Integrated graphics."),
        (true, true) => tr!("Integrated graphics, with the monitor connected to it."),
        (false, false) => tr!("A discrete card, which games run on."),
        (false, true) => {
            tr!("A discrete card, which games run on, with the monitor connected to it.")
        }
    };
    let description = if row.usable {
        kind.to_owned()
    } else {
        format!("{kind} {}", missing(adapters::find(cards, &row.id)))
    };
    DrawOn {
        title: row.name.clone(),
        description,
        key: row.id.clone(),
        usable: row.usable,
    }
}

/// Why Vulkan can't draw with an adapter: on Linux its Vulkan driver isn't
/// installed, and its vendor and kernel driver say which family of drivers
/// has it.
fn missing(card: Option<&Adapter>) -> &'static str {
    if !cfg!(target_os = "linux") {
        return tr!("Its driver doesn't offer Vulkan.");
    }
    let card = card.map(|card| (card.vendor, card.kernel_driver.as_str()));
    match card {
        Some((Vendor::Amd, _)) => {
            tr!(
                "Its Vulkan driver isn't installed: it needs Mesa's Vulkan driver for Radeon (RADV)."
            )
        }
        Some((Vendor::Intel, _)) => {
            tr!("Its Vulkan driver isn't installed: it needs Mesa's Vulkan driver for Intel (ANV).")
        }
        Some((Vendor::Nvidia, "nouveau")) => {
            tr!(
                "Its Vulkan driver isn't installed: it needs Mesa's Vulkan driver for NVIDIA (NVK)."
            )
        }
        Some((Vendor::Nvidia, _)) => tr!(
            "Its Vulkan driver isn't installed: it needs the Vulkan part of NVIDIA's driver, which comes with its utilities."
        ),
        _ => tr!("Its Vulkan driver isn't installed."),
    }
}

/// The adapter chosen by hand, which isn't in this computer now.
fn gone(id: &str) -> DrawOn {
    DrawOn {
        title: tr!("Graphics adapter {}", id.strip_prefix("pci:").unwrap_or(id)),
        description: tr!("It isn't in this computer now.").to_owned(),
        key: id.to_owned(),
        usable: false,
    }
}

/// What draws the interface now, and why the processor does where an
/// adapter was asked for. An adapter goes by its row's name, as the rows
/// above say it.
fn now(running: &Running, found: &[Row]) -> String {
    let drawing = match &running.adapter {
        Some(id) => {
            let listed = found.iter().find(|row| row.id == *id);
            let name = (listed.map(|row| row.name.as_str()))
                .or(running.name.as_deref())
                .unwrap_or(id.as_str());
            tr!("{} draws the interface now.", name)
        }
        None => tr!("The processor draws the interface now.").to_owned(),
    };
    match (&running.adapter, &running.why_not) {
        (None, Some(why)) if !why.is_empty() => format!("{drawing} {why}"),
        _ => drawing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This repository's owner's desktop, as the adapters list it: the
    /// 3080 drives the monitor; the Radeon draws once `usable`.
    fn rows(radeon: bool) -> Vec<Row> {
        vec![
            Row {
                id: "pci:0000:01:00.0".into(),
                name: "NVIDIA GeForce RTX 3080".into(),
                integrated: false,
                drives_display: true,
                usable: true,
            },
            Row {
                id: "pci:0000:73:00.0".into(),
                name: "AMD Radeon 610M".into(),
                integrated: true,
                drives_display: false,
                usable: radeon,
            },
        ]
    }

    /// The system's list: the Radeon on amdgpu, for its driver's family.
    fn cards() -> Vec<Adapter> {
        vec![Adapter {
            key: "pci:0000:73:00.0".into(),
            vendor: Vendor::Amd,
            device: 0x13c0,
            name: "AMD Radeon 610M".into(),
            integrated: true,
            drives_display: false,
            render_node: None,
            kernel_driver: "amdgpu".into(),
            suspended: false,
        }]
    }

    fn processor_draws(why: Option<&str>) -> Running {
        Running {
            adapter: None,
            name: None,
            why_not: why.map(str::to_owned),
        }
    }

    #[test]
    fn automatic_the_processor_and_each_adapter_by_key() {
        let drawing = build(None, &rows(true), &cards(), &processor_draws(None));
        let keys: Vec<&str> = drawing.rows.iter().map(|row| row.key.as_str()).collect();
        let all = ["auto", "processor", "pci:0000:01:00.0", "pci:0000:73:00.0"];
        assert_eq!(keys, all);
        assert_eq!(drawing.chosen, 0, "Automatic until chosen");
        assert!(drawing.rows[0].description.contains("AMD Radeon 610M"));
        let nvidia = &drawing.rows[2];
        assert!(nvidia.usable && nvidia.description.contains("games"));
        assert!(nvidia.description.contains("monitor"));
        for (saved, at) in [("auto", 0), ("processor", 1), ("pci:0000:73:00.0", 3)] {
            let drawing = build(Some(saved), &rows(true), &cards(), &processor_draws(None));
            assert_eq!(drawing.chosen, at, "{saved}");
        }
    }

    #[test]
    fn an_adapter_vulkan_cant_draw_with_says_which_driver_it_needs() {
        let drawing = build(None, &rows(false), &cards(), &processor_draws(None));
        let radeon = &drawing.rows[3];
        assert!(!radeon.usable);
        assert!(radeon.description.starts_with("Integrated graphics."));
        if cfg!(target_os = "linux") {
            assert!(
                radeon.description.contains("RADV"),
                "{}",
                radeon.description
            );
        }
        let automatic = &drawing.rows[0].description;
        assert!(automatic.contains("the processor, until AMD Radeon 610M can draw"));
        let none = build(None, &[], &[], &processor_draws(None));
        assert!(none.rows[0].description.contains("has none"));
        assert_eq!(none.rows.len(), 2);
    }

    #[test]
    fn an_adapter_chosen_and_gone_stays_chosen_and_faded() {
        let saved = Some("pci:0000:02:00.0");
        let drawing = build(saved, &rows(true), &cards(), &processor_draws(None));
        let gone = &drawing.rows[drawing.chosen];
        assert_eq!(drawing.chosen, 4);
        assert_eq!(gone.title, "Graphics adapter 0000:02:00.0");
        assert!(!gone.usable);
    }

    #[test]
    fn the_line_says_what_draws_now_and_why_not_a_gpu() {
        let found = rows(true);
        let on_radeon = Running {
            adapter: Some("pci:0000:73:00.0".into()),
            name: None,
            why_not: None,
        };
        let line = build(None, &found, &[], &on_radeon).line;
        assert_eq!(line, "AMD Radeon 610M draws the interface now.");
        let why = "No integrated graphics can draw the interface.";
        let line = build(None, &found, &[], &processor_draws(Some(why))).line;
        assert_eq!(
            line,
            format!("The processor draws the interface now. {why}")
        );
        let line = build(Some("processor"), &found, &[], &processor_draws(None)).line;
        assert_eq!(line, "The processor draws the interface now.");
    }
}
