//! The main window: how it opens; what of it reaches `update` (its focus,
//! its frames and its close button); and its sight, whether the desktop
//! paints it, which says what in it keeps going ([`sight`]).

pub mod seen;

use std::time::Instant;

use iced::event::{self, Event};
use iced::{Size, Subscription, Task, window};
use yapfast_core::application::Action;

use anomi_theme::metrics;
use anomi_theme::motion::Background;

use crate::app::App;
use crate::intent;
use crate::intent::chat::composer::Done;
use crate::message::Message;
use crate::project::Reach;

/// The desktop entry's id, so the desktop finds the window's icon; the Slint
/// app's.
pub const APP_ID: &str = anomi_desktop::icon::APP_ID;

/// How the main window opens: the test display's size under it, else the
/// first start's.
pub fn settings() -> window::Settings {
    window::Settings {
        size: test_size().unwrap_or(Size::new(metrics::WINDOW_WIDTH, metrics::WINDOW_HEIGHT)),
        min_size: Some(Size::new(
            metrics::WINDOW_MIN_WIDTH,
            metrics::WINDOW_MIN_HEIGHT,
        )),
        // The close button asks; `update` decides: to the tray, or quit.
        exit_on_close_request: false,
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: APP_ID.into(),
            ..Default::default()
        },
        ..window::Settings::default()
    }
}

/// `YAPFAST_UI_TEST_SIZE`, as `scripts/ui-session` sets it, on its private
/// display alone.
pub fn test_size() -> Option<Size> {
    if std::env::var("YAPFAST_FRAMEWORK_ISOLATED_DISPLAY").as_deref() != Ok("1") {
        return None;
    }
    let size = std::env::var("YAPFAST_UI_TEST_SIZE").ok()?;
    let (width, height) = size.split_once('x')?;
    let (width, height) = (width.parse::<f32>().ok()?, height.parse::<f32>().ok()?);
    (width >= metrics::WINDOW_MIN_WIDTH && height >= metrics::WINDOW_MIN_HEIGHT)
        .then(|| Size::new(width, height))
}

/// The window's focus, its size, its display's scale, Shift, files dragged
/// over it, its close button, its keys for keybinds (handed on with no
/// message, `platform::keybinds::keys`), input, which says someone uses it
/// (`clocks::away`), and the frames it draws, which say whether the desktop
/// paints it ([`seen`]): one filter for all, as each wakes iced's executor
/// for every event, frames included. Nothing here ticks: each fires only as
/// the desktop says something happened, and a frame brings a message only
/// while the window is unseen.
pub fn subscription() -> Subscription<Message> {
    Subscription::batch([
        event::listen_raw(focus),
        window::close_requests().map(Message::CloseRequested),
    ])
}

fn focus(event: Event, _status: event::Status, _window: window::Id) -> Option<Message> {
    // A frame drawn, which neither the keybinds nor `away` see: counted as
    // heard, not by iced's stamp, which rounds up to the display's refresh.
    if let Event::Window(window::Event::RedrawRequested(_)) = &event {
        return seen::drawn(Instant::now()).map(Message::Sight);
    }
    if let Some(recorded) = crate::platform::keybinds::keys::heard(&event) {
        return Some(recorded);
    }
    if let Some(used) = crate::clocks::away::input(&event) {
        return Some(used);
    }
    match event {
        Event::Window(window::Event::Focused) => Some(Message::Focused(true)),
        Event::Window(window::Event::Unfocused) => Some(Message::Focused(false)),
        Event::Window(window::Event::Rescaled(scale)) => Some(Message::Rescaled(scale)),
        Event::Window(window::Event::Resized(size)) => Some(Message::Resized(size)),
        // Files dragged over the window and dropped, as winit tells of
        // them (X11, Windows): the composer's next message takes them.
        Event::Window(window::Event::FileHovered(_)) => Some(dragged(Done::Hovered(true))),
        Event::Window(window::Event::FilesHoveredLeft) => Some(dragged(Done::Hovered(false))),
        Event::Window(window::Event::FileDropped(path)) => Some(dragged(Done::Dropped(path))),
        Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) => {
            Some(Message::Shift(modifiers.shift()))
        }
        _ => None,
    }
}

fn dragged(done: Done) -> Message {
    Message::Composer(done)
}

/// The window came to the front or went behind another window. What asks
/// for attention follows the focus: the core hears, which asks Discord for
/// the Activity panel's feed only in front, as Discord's client does. The
/// core is told the window is behind as the app starts (`App::new`), until
/// the window says otherwise. Whether the newest messages are being read
/// follows the focus too: the core hears it here as well as with each
/// report of the rows in view. In front, the taskbar stops flashing;
/// behind, what hangs off the window closes, menus, popovers, cards and
/// pickers, as in Discord's client (`intent::shell::unfocused`). Only the
/// window's own `Unfocused` brings that, and closing to the tray, which
/// closes them all anyway. What keeps going follows the window's [`sight`],
/// which the focus is a part of.
pub fn focused(app: &mut App, front: bool) -> Task<Message> {
    tracing::debug!(front, "the window's focus");
    if !front {
        crate::platform::keybinds::record::unfocused(app);
    }
    app.vm.chrome.focused = front;
    if std::mem::replace(&mut app.motion.front, front) == front {
        return Task::none();
    }
    let task = app.dispatch(Action::WindowFocused(front));
    let attention = if front {
        // The taskbar's flash for a message that came meanwhile stops.
        crate::effects::notify::front(app)
    } else {
        intent::shell::unfocused(app)
    };
    let sighted = sight(app);
    Task::batch([task, attention, sighted, intent::chat::reading(app)])
}

/// What the desktop says of whether it paints the window, besides the
/// focus and the tray (`App::hidden`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Sighting {
    /// The desktop stops drawing every window it doesn't paint, so the
    /// frames tell a covered window from one on another monitor: only the
    /// test display (`platform::window::detect`).
    pub exact: bool,
    /// Minimised, which Windows tells with a size of 0.
    pub minimised: bool,
    /// A redraw asked a second ago wasn't drawn, and no two frames came
    /// close together since ([`seen`]).
    pub unpainted: bool,
    /// A watcher of the desktop's says a window covers it; none yet.
    pub covered: bool,
}

impl Sighting {
    /// The desktop doesn't paint the window.
    fn hides(self) -> bool {
        self.minimised || self.unpainted || self.covered
    }
}

/// Where the window stands, again: in front (focused and painted), on
/// screen (painted, not focused) or hidden (not painted: in the tray,
/// minimised, covered, on another workspace, the screen off), and what
/// keeps going there (`Motion::live`): in front everything, on screen what
/// the setting says ([`background`]), hidden nothing. As the window turns
/// live every clock says the time again, and either way the call's
/// pictures are placed again and the video followed or let go. The clocks'
/// waits and what the backend's gate holds follow after every update
/// (`clocks::arm`, `backend::follow`).
pub fn sight(app: &mut App) -> Task<Message> {
    let was = app.vm.chrome.live;
    let sighting = app.sighting;
    app.motion.unseen = app.hidden || sighting.hides();
    // As chosen in Settings' Advanced, else this desktop's default.
    app.motion.background = background(app.layout.background_updates, sighting.exact);
    let live = app.motion.live();
    app.vm.chrome.live = live;
    tracing::debug!(sight = ?app.motion.sight(), live, "the window's sight");
    if live != was {
        if live {
            crate::clocks::again(app);
        }
        app.show(Reach::VOICE | Reach::VIDEO);
    }
    Task::none()
}

/// What a window on screen but not focused keeps doing: as chosen; else
/// what matters where the desktop says when a game covers the window, and
/// nothing where it can't, as a window behind a game would.
pub fn background(chosen: Option<Background>, exact: bool) -> Background {
    match (chosen, exact) {
        (Some(chosen), _) => chosen,
        (None, true) => Background::Important,
        (None, false) => Background::Nothing,
    }
}

/// What reached `update` of the window's sight (`Message::Sight`): from its
/// frames ([`seen`]), and from the desktop.
pub fn heard(app: &mut App, said: seen::Heard) -> Task<Message> {
    match said {
        seen::Heard::Unseen => {
            tracing::debug!("the window isn't painted");
            app.sighting.unpainted = true;
        }
        // The frames' task woke this update, whose start judged ([`judge`]):
        // the probe turns only there, so no frame drawn since is missed.
        seen::Heard::Late => return Task::none(),
        // A configure's lone frame, maybe: the update this message brings
        // asks a redraw, which only a painted window draws.
        seen::Heard::First => {
            tracing::debug!("a frame drawn while unpainted");
            return Task::none();
        }
        seen::Heard::Seen => {
            tracing::debug!("the window is painted again");
            app.sighting.unpainted = false;
        }
        seen::Heard::Detected(exact) => {
            tracing::debug!(exact, "whether the desktop tells when it paints the window");
            app.sighting.exact = exact;
        }
    }
    sight(app)
}

/// As every update starts: a redraw asked [`seen::UNPAINTED`] ago and not
/// drawn makes the shown window unseen at once.
pub fn judge(app: &mut App) -> Task<Message> {
    if shown(app) && seen::judge(Instant::now()) {
        return heard(app, seen::Heard::Unseen);
    }
    Task::none()
}

/// As every update ends: iced asks the shown window for a redraw after it.
pub fn asked(app: &App) {
    if shown(app) {
        seen::asked(Instant::now());
    }
}

/// A window is made again, out of the tray: painted until its frames say
/// otherwise, whatever the one before last heard, and not minimised.
pub fn made(app: &mut App) {
    seen::fresh();
    app.sighting.unpainted = false;
    app.sighting.minimised = false;
}

/// A window is open and not closed to the tray. Tests draw no frame, so
/// there every redraw asked would stay undrawn: the watchdog doesn't run.
fn shown(app: &App) -> bool {
    !cfg!(test) && app.window.is_some() && !app.hidden
}

#[cfg(test)]
mod tests;
