//! The games this device names itself, which the desktop sets from its
//! settings: those named by hand, and those chosen from Discord's list in
//! Settings' picker. They're the device's, not an account's, so every
//! session shares them, and a change reaches the detection running.
use std::sync::OnceLock;
use tokio::sync::watch;

/// An executable, by file name or full path, and the game to show for it.
/// One chosen from Discord's list names its application, as a game Discord
/// detects does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CustomGame {
    /// Empty for a game shown without a program: it shows while sharing is
    /// on, from [`Self::since`], until it's hidden or gone.
    pub executable: String,
    pub name: String,
    /// The game's application on Discord, which its activity names.
    pub id: Option<u64>,
    /// Its activity is off: its program still counts as this game, so
    /// neither it nor Discord's list shows anything for it.
    pub hidden: bool,
    /// When a game shown without a program was turned on, in Unix seconds;
    /// its activity says it has been played since then.
    pub since: Option<u64>,
}

fn channel() -> &'static watch::Sender<Vec<CustomGame>> {
    static CHANNEL: OnceLock<watch::Sender<Vec<CustomGame>>> = OnceLock::new();
    CHANNEL.get_or_init(|| watch::channel(Vec::new()).0)
}

pub fn set(games: Vec<CustomGame>) {
    channel().send_if_modified(|old| {
        if *old == games {
            false
        } else {
            *old = games;
            true
        }
    });
}

pub(crate) fn subscribe() -> watch::Receiver<Vec<CustomGame>> {
    channel().subscribe()
}
