//! What draws, from the choice, the system's adapters, whose Vulkan drivers
//! are installed, and the failures in a row so far: as the program starts,
//! and again as Settings chooses. Automatic is the owner's rule: the
//! integrated graphics, so a discrete card stays with the game, else the
//! processor; a discrete card draws only when chosen by hand. Reading this
//! loads no driver: the adapters are sysfs's (`yapfast_core::adapters`) and
//! the drivers their manifests ([`super::drivers`]).

use iced_wgpu::adapter::Choice as Wanted;
use yapfast_core::adapters::Adapter;
use yapfast_lang::tr;

use super::fail::{Failure, LIMIT, next};
use super::{Choice, drivers, list};

/// What a start, or a choice in Settings, draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// The choice in effect.
    pub(super) choice: Choice,
    /// The adapter asked of wgpu; none while the processor draws.
    pub(super) target: Option<Target>,
    /// Why the processor draws, where a GPU was asked for.
    pub(super) why: Option<Why>,
    /// The window says why too: a choice made by hand that can't be met.
    pub(super) notice: bool,
    /// The globs naming the Vulkan drivers' manifests this process may load
    /// (`VK_LOADER_DRIVERS_SELECT`): the adapter's that draws; with none,
    /// Automatic's, else every adapter's, for a switch while it runs.
    pub(super) drivers: Vec<&'static str>,
}

/// The adapter asked of wgpu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Target {
    pub(super) wanted: Wanted,
    /// Its key in `yapfast_core::adapters`, and its name there.
    pub(super) id: String,
    pub(super) name: String,
}

/// Why the processor draws, said in Settings and, for a choice made by hand
/// or a failure, over the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Why {
    /// Automatic, on a computer without integrated graphics, or with none
    /// Vulkan offers.
    NoIntegrated,
    /// The adapter's Vulkan driver isn't installed (Settings' row of it
    /// names the one it needs).
    NoDriver,
    /// The adapter chosen by hand isn't in the computer now.
    Gone,
    /// It failed three times in a row: tried again once chosen again.
    Stopped,
    /// The app closed while a window opened on it, before it had drawn
    /// there a few seconds: tried again once chosen again.
    Died(String),
    /// Chosen while running, its driver wasn't let load at the start.
    NextStart(String),
    /// It failed: how, and whether it was the third time in a row.
    Failed {
        failure: Failure,
        name: String,
        last: bool,
    },
}

impl Why {
    /// In the chosen language, as Settings shows it after "The processor
    /// draws the interface now."
    pub(super) fn text(&self) -> String {
        match self {
            Self::NoIntegrated => tr!("No integrated graphics can draw the interface.").to_owned(),
            Self::NoDriver => tr!("The graphics adapter's Vulkan driver isn't installed.").to_owned(),
            Self::Gone => tr!("The graphics adapter chosen isn't in this computer now.").to_owned(),
            Self::Stopped => tr!(
                "The graphics adapter failed three times in a row, so the processor draws until it's chosen again."
            )
            .to_owned(),
            Self::NextStart(name) => tr!(
                "{} draws from the next start, when its driver is loaded.",
                name
            ),
            Self::Died(name) => format!(
                "{} {}",
                tr!("The app closed as {} started to draw the window.", name),
                tr!("It's tried again only once it's chosen again in Settings.")
            ),
            Self::Failed {
                failure,
                name,
                last,
            } => format!("{} {}", failure.text(name), next(*last)),
        }
    }
}

/// What `choice` draws with. `installed` says whether an adapter's Vulkan
/// driver is; after [`LIMIT`] failures in a row the processor draws, unless
/// the command line asks (`forced`).
pub(super) fn decide(
    choice: &Choice,
    cards: &[Adapter],
    installed: impl Fn(&Adapter) -> bool,
    failures: u8,
    forced: bool,
) -> Plan {
    let integrated = || cards.iter().filter(|card| card.integrated);
    let auto = integrated()
        .filter(|&card| installed(card))
        .chain(integrated())
        .next();
    let mut plan = Plan {
        choice: choice.clone(),
        target: None,
        why: None,
        notice: false,
        drivers: match auto {
            Some(card) => globs_of([card]),
            None => globs_of(cards),
        },
    };
    let by_hand = matches!(choice, Choice::Adapter(_));
    let card = match choice {
        Choice::Processor => return plan,
        Choice::Auto => match auto {
            Some(card) => card,
            None => {
                plan.why = Some(Why::NoIntegrated);
                return plan;
            }
        },
        Choice::Adapter(id) => match cards.iter().find(|card| card.key == *id) {
            Some(card) => card,
            None => {
                plan.why = Some(Why::Gone);
                plan.notice = true;
                return plan;
            }
        },
    };
    plan.drivers = globs_of([card]);
    if !installed(card) {
        plan.why = Some(Why::NoDriver);
        plan.notice = by_hand;
        return plan;
    }
    if failures >= LIMIT && !forced {
        plan.why = Some(Why::Stopped);
        return plan;
    }
    let wanted = if by_hand {
        Wanted::Bus(bus(card))
    } else {
        Wanted::Auto
    };
    plan.target = Some(Target {
        wanted,
        id: card.key.clone(),
        name: card.name.clone(),
    });
    plan
}

/// A start that died on the adapter `trying` names (its id, written before
/// a window opened on it and cleared once it had drawn there a while,
/// [`super::opening`]) leaves the processor drawing, and the window saying
/// why, until a GPU is chosen again; the command line (`forced`) still
/// tries it. One death is enough: a crash at every start can't be told from
/// a crash once, and a Wayland compositor's protocol error kills the
/// process before it can count anything.
pub(super) fn died(plan: &mut Plan, trying: Option<&str>, forced: bool) {
    let Some(target) = (plan.target).take_if(|target| !forced && trying == Some(&*target.id))
    else {
        return;
    };
    plan.why = Some(Why::Died(target.name));
    plan.notice = true;
}

/// The globs of these adapters' Vulkan drivers, each once.
fn globs_of<'a>(cards: impl IntoIterator<Item = &'a Adapter>) -> Vec<&'static str> {
    let mut globs: Vec<&'static str> = Vec::new();
    for glob in cards
        .into_iter()
        .flat_map(|card| drivers::globs(card).iter().copied())
    {
        if !globs.contains(&glob) {
            globs.push(glob);
        }
    }
    globs
}

/// How wgpu finds the adapter: its PCI address, or where the system gives
/// none (Windows' LUID keys), its vendor and device ids.
fn bus(card: &Adapter) -> String {
    if let Some(address) = card.pci_address() {
        return address.to_ascii_lowercase();
    }
    match list::vendor_id(card.vendor) {
        Some(vendor) => format!("{vendor:04x}:{:04x}", card.device),
        None => card.key.clone(),
    }
}
