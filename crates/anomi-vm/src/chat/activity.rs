//! A message's activity: a Discord invite to a party, or a request to
//! stream, as Discord's client shows it on a card. What the card shows and
//! what it asks for.

use crate::kind::Expected;

/// What a message's activity asks this account for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// It asks this account to stream, by the requester's name.
    #[default]
    StreamRequest,
}

/// The card a message's activity shows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Activity {
    pub kind: Kind,
    /// The activity's name, such as "Dota 2".
    pub name: String,
    /// The application's picture, once it arrived.
    pub icon: Expected,
    /// Who asked, for "User {} wants you to stream".
    pub requester: String,
    /// Too old to take: its buttons give way to "The stream request
    /// expired.".
    pub expired: bool,
    /// Accepted or declined: its buttons went.
    pub taken: bool,
}

/// What the card asks for; the message's row key rides in each, so the app
/// finds the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Intent {
    /// Accept: the way to start the stream opens (the Go Live dialog).
    Accept(u64),
    /// Decline: the card's buttons go.
    Decline(u64),
}
