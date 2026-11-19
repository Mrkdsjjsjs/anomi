//! What draws the window: the processor (iced's tiny-skia) or a graphics
//! adapter (wgpu on Vulkan, `vendor/iced_wgpu`). Settings' Advanced chooses
//! ([`Choice`]) and `layout.json` keeps it as `interface_renderer`; how it's
//! chosen, switched and given up is in docs/iced/README.md, "Choosing what
//! draws".
//!
//! - As the program starts ([`prepare`], [`begin`]), before any thread, the
//!   Vulkan loader is told to load only the driver of the adapter that may
//!   draw (`VK_LOADER_DRIVERS_SELECT`, [`drivers`]); `run` then enables wgpu
//!   with that adapter, or leaves it off.
//! - Automatic is the owner's rule: the integrated graphics, else the
//!   processor; a discrete card only when chosen by hand ([`plan`]).
//! - Settings lists the adapters a child process found ([`adapters`]) and
//!   [`choose`] switches at once, through `iced::backend::configure`
//!   ([`switch`]).
//! - Its frames go straight into the window only on the adapter the desktop
//!   composites on ([`screen`]); any other's are read back and shown
//!   through shared memory (`iced_wgpu::adapter::Path`).
//! - A GPU that doesn't start or fails under the window gives way to the
//!   processor with a notice, and three failures in a row keep the
//!   processor until a GPU is chosen again ([`fail`]); so does one start
//!   that died before its window had drawn a few seconds ([`opening`]).
//!
//! Only `run` makes any of this live: the tests never start a GPU, and in
//! them [`choose`] keeps the choice alone.

mod choice;
mod drivers;
mod fail;
mod list;
mod plan;
mod screen;
mod switch;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use iced::futures::{Stream, stream};
use iced::{Subscription, Task, window};
use iced_wgpu::adapter::Info;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use yapfast_core::adapters::Adapter;

use crate::app::App;
use crate::layout::{Layout, Saved};
use crate::message::Message;
use crate::project::Reach;

pub use choice::{AUTO, Choice, Override, PROCESSOR, Row, Running, adapter_id};
pub use list::{ARGUMENT as LIST_ADAPTERS, print as print_adapters};
pub use plan::Plan;
pub use switch::{choose, settings};

use plan::Why;

/// What reaches `update` of the GPU.
#[derive(Clone, Debug)]
pub enum Due {
    /// The window opened: its renderer was made.
    Opened,
    /// A frame was drawn since, while a try written ahead or failures
    /// counted wait to be settled.
    Drew,
    /// The start draws on the processor where a choice made by hand can't
    /// be met.
    Unmet,
    /// The window's GPU failed under it, as a renderer's thread told.
    Lost(iced_wgpu::lost::Lost),
    /// The child process listed the adapters.
    Listed,
    /// iced made the renderer asked for, or couldn't make any: why.
    Switched(Option<String>),
    /// A few seconds passed since the window drew on the GPU, or gave it
    /// up, in the try of this number: it didn't die there.
    Settled(u32),
}

/// This process runs the window (`run` began it), not a test.
static LIVE: AtomicBool = AtomicBool::new(false);
/// The hook heard of a failure since a GPU was last chosen.
static FAILED: AtomicBool = AtomicBool::new(false);
/// The window's first frame on the GPU is waited for.
static FIRST_FRAME: AtomicBool = AtomicBool::new(false);
static TELL: Mutex<Option<UnboundedSender<Due>>> = Mutex::new(None);
static HEARD: Mutex<Option<UnboundedReceiver<Due>>> = Mutex::new(None);
static STATE: Mutex<State> = Mutex::new(State::new());

struct State {
    /// The choice in effect: Settings', or the command line's for this run.
    asked: Choice,
    /// The adapter wgpu was asked for, by id and name, while it may draw.
    target: Option<(String, String)>,
    /// Why the processor draws, while it does.
    why: Option<Why>,
    /// `renderer_trying` names the adapter a window opened on in this run,
    /// not settled yet.
    trying: bool,
    /// Which try that is, so a settling meant for an earlier one is ignored.
    epoch: u32,
    /// A window drew on the GPU a while in this run: windows opened again
    /// on it write nothing ahead.
    settled: bool,
    /// The system's adapters as last read, for their names.
    cards: Vec<Adapter>,
}

impl State {
    const fn new() -> Self {
        Self {
            asked: Choice::Auto,
            target: None,
            why: None,
            trying: false,
            epoch: 0,
            settled: false,
            cards: Vec::new(),
        }
    }
}

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn live() -> bool {
    LIVE.load(Ordering::Acquire)
}

/// Hands `update` news from another thread; nothing before `begin`.
fn tell(due: Due) {
    if let Some(tell) = TELL.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
        let _ = tell.send(due);
    }
}

/// What draws as the program starts, from Settings' choice, the command
/// line's, the failures so far and a start that died; lets only that
/// adapter's Vulkan driver load. `cli::main` calls it before starting any
/// thread.
pub fn prepare(saved: &Saved, asked: Option<&Override>) -> Plan {
    let kept = Choice::parse(saved.interface_renderer.as_deref());
    let choice = match asked {
        Some(asked) => asked.choice(&kept),
        None => kept,
    };
    let cards = yapfast_core::adapters::list();
    let failures = saved.renderer_failures;
    let mut plan = plan::decide(
        &choice,
        &cards,
        drivers::installed,
        failures,
        asked.is_some(),
    );
    plan::died(&mut plan, saved.renderer_trying.as_deref(), asked.is_some());
    drivers::select(&plan.drivers);
    state().cards = cards;
    plan
}

/// Makes the plan live before the window opens: the adapter asked of wgpu,
/// the adapter the desktop composites on, and the hook its failures reach
/// `update` through. Whether wgpu may start, which `run` enables.
pub fn begin(plan: Plan) -> bool {
    LIVE.store(true, Ordering::Release);
    let (told, heard) = unbounded_channel();
    *TELL.lock().unwrap_or_else(PoisonError::into_inner) = Some(told);
    *HEARD.lock().unwrap_or_else(PoisonError::into_inner) = Some(heard);
    iced_wgpu::lost::set_hook(|lost| {
        FAILED.store(true, Ordering::Release);
        tell(Due::Lost(lost.clone()));
    });
    let gpu = plan.target.is_some();
    match &plan.target {
        Some(target) => {
            iced_wgpu::adapter::choose(target.wanted.clone());
            let cards = state().cards.clone();
            iced_wgpu::adapter::set_display_adapter(screen::adapter(&cards));
            tracing::info!(adapter = %target.name, "the window is to draw on the graphics adapter");
        }
        None => tracing::info!(why = ?plan.why, "the window draws on the processor"),
    }
    if plan.notice {
        tell(Due::Unmet);
    }
    let mut state = state();
    state.asked = plan.choice;
    state.target = plan.target.map(|target| (target.id, target.name));
    state.why = plan.why;
    gpu
}

/// A window is about to open: while a GPU may draw and hasn't drawn a
/// while in this run, the adapter is written ahead as `renderer_trying`,
/// on disk before the window and its renderer are made. If the app dies
/// before it's cleared ([`Due::Settled`]), by a crash, an abort or a
/// Wayland protocol error, which no code of its runs after, the next start
/// finds it and draws on the processor (`plan::died`).
pub fn opening(layout: &mut Layout) {
    if !live() || !iced_wgpu::enabled::is_enabled() {
        return;
    }
    let state = state();
    let Some((id, _name)) = state.target.clone().filter(|_| !state.settled) else {
        return;
    };
    drop(state);
    mark(layout, &id);
}

/// The person quits: a try not settled yet is no death, and isn't left
/// written ahead; then whatever waits of the layout is written.
pub fn quitting(layout: &mut Layout) {
    let trying = std::mem::take(&mut state().trying);
    if trying && layout.renderer_trying.is_some() {
        layout.edit(|saved| saved.renderer_trying = None);
    }
    layout.flush();
}

/// Writes `id` ahead as the adapter tried, and waits until it's on disk.
fn mark(layout: &mut Layout, id: &str) {
    if layout.renderer_trying.as_deref() != Some(id) {
        layout.edit(|saved| saved.renderer_trying = Some(id.to_owned()));
    }
    layout.flush();
    let mut state = state();
    state.trying = true;
    state.epoch = state.epoch.wrapping_add(1);
}

/// The adapters Settings offers: listed once, in a child process, as
/// listing loads every Vulkan driver; until that's done, the system's own
/// list, each usable where its Vulkan driver is installed.
pub fn adapters() -> Arc<[Row]> {
    list::rows()
}

/// What draws the window now.
pub fn running() -> Running {
    let active = iced_wgpu::adapter::active();
    let state = state();
    match active {
        Some(info) => {
            let id = id_of(&info, &state.cards);
            let card = state.cards.iter().find(|card| card.key == id);
            let name = card.map_or_else(|| info.name.clone(), |card| card.name.clone());
            Running {
                adapter: Some(id),
                name: Some(name),
                why_not: None,
            }
        }
        None => Running {
            adapter: None,
            name: None,
            why_not: state.why.as_ref().map(Why::text),
        },
    }
}

/// The id Settings knows a Vulkan adapter by.
fn id_of(info: &Info, cards: &[Adapter]) -> String {
    match cards.iter().find(|card| list::same(card, info)) {
        Some(card) => card.key.clone(),
        None if info.bus.is_empty() => info.ids(),
        None => format!("pci:{}", info.bus.to_ascii_lowercase()),
    }
}

/// Opened windows, the failures' news and, while it's waited for, the first
/// frame on the GPU; nothing outside the program `run` starts.
pub fn subscription() -> Subscription<Message> {
    if !live() {
        return Subscription::none();
    }
    let heard = Subscription::run(heard);
    let opened = window::open_events().map(|_id| Message::Gpu(Due::Opened));
    if !FIRST_FRAME.load(Ordering::Relaxed) {
        return Subscription::batch([heard, opened]);
    }
    let drew = window::frames().map(|_at| Message::Gpu(Due::Drew));
    Subscription::batch([heard, opened, drew])
}

/// What the renderer's threads and the lister tell, as they tell it; it
/// waits on the channel and wakes for nothing else.
fn heard() -> impl Stream<Item = Message> {
    let heard = HEARD.lock().unwrap_or_else(PoisonError::into_inner).take();
    // Taken already, or never made: a channel that ends at once.
    let heard = heard.unwrap_or_else(|| unbounded_channel::<Due>().1);
    stream::unfold(heard, |mut heard| async move {
        heard.recv().await.map(|due| (Message::Gpu(due), heard))
    })
}

pub fn due(app: &mut App, due: Due) -> Task<Message> {
    match due {
        Due::Opened => fail::opened(app),
        Due::Drew => fail::drew(),
        Due::Unmet => {
            let why = state().why.clone();
            if let Some(why) = why {
                app.vm.sign_in.graphics_notice = Some(fail::notice(&why.text(), None));
            }
            Task::none()
        }
        Due::Lost(lost) => fail::lost(app, &lost),
        Due::Listed => {
            app.shown.settings.kept.changed();
            app.show(Reach::SETTINGS);
            Task::none()
        }
        Due::Switched(error) => fail::switched(app, error),
        Due::Settled(epoch) => fail::settled(app, epoch),
    }
}
