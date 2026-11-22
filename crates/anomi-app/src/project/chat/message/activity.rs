//! A message's activity card: what Discord's request to stream shows and
//! whether it can still be taken. Part of C07 (docs/iced/tasks/chat.md);
//! the card is beside a message's poll.

use yapfast_core::chat::{ChatMessage, MessageActivityKind};

use anomi_vm::chat::activity::{Activity, Kind};

use super::{Context, pictures};

/// How long a request to stream may be taken, as Discord's client keeps
/// it: ten minutes from the message's snowflake.
const REQUEST_SECS: u64 = 10 * 60;

/// The card a message's activity shows: a request to stream, with its
/// requester, whether it's still fresh, and whether it was taken.
pub(super) fn card(cx: &Context<'_>, message: &ChatMessage) -> Option<Activity> {
    let activity = message.activity.as_ref()?;
    if activity.kind != MessageActivityKind::StreamRequest {
        return None;
    }
    Some(Activity {
        kind: Kind::StreamRequest,
        name: activity.name.clone(),
        icon: match pictures::find(cx.state, activity.icon.as_deref()) {
            Some(handle) => anomi_vm::kind::Expected::Ready(handle),
            None if activity.icon.is_some() => anomi_vm::kind::Expected::Waiting,
            None => anomi_vm::kind::Expected::None,
        },
        requester: message.author.clone(),
        expired: message.timestamp != 0 && message.timestamp + REQUEST_SECS <= cx.now,
        taken: cx.kept.taken_activities.contains(&message.item),
    })
}
