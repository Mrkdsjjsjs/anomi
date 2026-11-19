//! What Settings and the command line choose, what Settings lists, and what
//! it's told draws now.

use serde::{Deserialize, Serialize};

/// `interface_renderer`'s keys for Automatic and the processor; an
/// adapter's is its id.
pub const AUTO: &str = "auto";
pub const PROCESSOR: &str = "processor";

/// What the person chose to draw the window with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    /// The integrated graphics where they can draw, else the processor;
    /// never a discrete card.
    #[default]
    Auto,
    Processor,
    /// The adapter of this id, as `yapfast_core::adapters` keys it
    /// (`"pci:0000:73:00.0"`), chosen by hand.
    Adapter(String),
}

impl Choice {
    /// From `interface_renderer`: unset, empty or `"auto"` is Automatic.
    pub fn parse(saved: Option<&str>) -> Self {
        match saved.map(str::trim) {
            None | Some("" | AUTO) => Self::Auto,
            Some(PROCESSOR | "software") => Self::Processor,
            Some(id) => Self::Adapter(id.to_owned()),
        }
    }

    /// What `interface_renderer` keeps for it.
    pub fn saved(&self) -> Option<String> {
        Some(match self {
            Self::Auto => AUTO.to_owned(),
            Self::Processor => PROCESSOR.to_owned(),
            Self::Adapter(id) => id.clone(),
        })
    }
}

/// An adapter Settings offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    /// Its key in `yapfast_core::adapters`, which `interface_renderer` keeps.
    pub id: String,
    /// Its name from the system's list of PCI ids ("AMD Radeon 610M").
    pub name: String,
    pub integrated: bool,
    /// A monitor is plugged into it.
    pub drives_display: bool,
    /// Vulkan offers it, so it can draw the window.
    pub usable: bool,
}

/// What draws the window now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Running {
    /// The adapter's id; none while the processor draws.
    pub adapter: Option<String>,
    pub name: Option<String>,
    /// While the processor draws: why, where a GPU was asked for, or what a
    /// restart would still change.
    pub why_not: Option<String>,
}

/// What the command line asks to draw with for one run, over what Settings
/// keeps (`--renderer`, `--adapter`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Override {
    Auto,
    /// The adapter Settings chose by hand, else Automatic's.
    Gpu,
    Software,
    Adapter(String),
}

impl Override {
    /// The choice for this run, given Settings'.
    pub(super) fn choice(&self, kept: &Choice) -> Choice {
        match self {
            Self::Auto => Choice::Auto,
            Self::Software => Choice::Processor,
            Self::Adapter(id) => Choice::Adapter(id.clone()),
            Self::Gpu => match kept {
                Choice::Adapter(id) => Choice::Adapter(id.clone()),
                Choice::Auto | Choice::Processor => Choice::Auto,
            },
        }
    }
}

/// `--adapter`'s argument as `yapfast_core::adapters` keys it: a bare PCI
/// address (`0000:73:00.0`) gets its `pci:`.
pub fn adapter_id(given: &str) -> String {
    let given = given.trim();
    if given.starts_with("pci:") || given.starts_with("luid:") {
        given.to_owned()
    } else {
        format!("pci:{}", given.to_ascii_lowercase())
    }
}
