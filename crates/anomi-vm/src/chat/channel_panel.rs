//! A voice channel's chat, open beside its stage: what its header asks
//! for. The rows and the composer are the channel's own (the timeline and
//! the composer over the selected room), so nothing else lives here.

/// What the panel's header asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Intent {
    /// The cross: the stage fills the body again (`Action::ShowVoiceChat(false)`).
    Close,
}
