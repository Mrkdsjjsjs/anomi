//! The message the pointer is on: lit, with its bar, and its author's
//! moving picture and decoration, which their view plays while it's lit, as
//! Discord's client plays them under the pointer. They're asked for only
//! then, and watched by the row, so their coming builds it again; a row
//! that goes dark takes its stills back. Every other row holds its stills
//! and asks for nothing moving.

use yapfast_core::application::Application;

use anomi_vm::chat::message::Bar;
use anomi_vm::chat::timeline::Timeline;

use super::{Shown, moved};
use crate::pictures::Requests;
use crate::project::chat::message;

/// The message the pointer is on lies lit with its bar, and the one asked
/// to be confirmed deleted keeps its bar with the question; any other lies
/// as it is. The lit one's author shows their moving pictures once they
/// came, asked for here as the update ends (`App::update`).
pub(crate) fn light(
    core: &Application,
    timeline: &mut Timeline,
    shown: &mut Shown,
    requests: &mut Requests,
) {
    let state = &core.state;
    let Some(look) = &shown.look else {
        return;
    };
    let confirming = state
        .confirm_delete
        .as_deref()
        .and_then(|item| moved::key_of_item(&shown.built.messages, item));
    let mut moving: Vec<(u64, String)> = Vec::new();
    for (row, message) in timeline.rows.iter_mut().zip(shown.built.messages.iter()) {
        let lit = shown.lit == Some(row.key);
        let barred = (lit || confirming == Some(row.key)) && row.message.acts();
        // The thread beside, narrower, offers two reactions.
        let bar = barred.then(|| {
            let mut bar = message::bar(core, look, message);
            bar.frequent.truncate(if timeline.aside { 2 } else { 3 });
            Bar {
                shift: shown.shift,
                ..bar
            }
        });
        let made = &mut row.message;
        // Only a row that shows its author's picture, lit or going dark.
        let pictured = made.headed() && !made.compact;
        if (lit || made.lit) && pictured && !message::collapsed(message, &shown.revealed) {
            let faces = message::faces(state, look, message, lit);
            if made.author.avatar != faces.avatar {
                made.author.avatar = faces.avatar;
            }
            if made.author.decoration != faces.decoration {
                made.author.decoration = faces.decoration;
            }
            moving.extend(faces.moving.into_iter().map(|address| (row.key, address)));
        }
        if made.lit != lit {
            made.lit = lit;
            made.attachments.lit = lit;
        }
        if made.bar != bar {
            made.bar = bar;
        }
    }
    for (key, address) in &moving {
        shown.pictures.note(state, address, *key);
    }
    requests.want(state, moving.iter().map(|(_, address)| address.as_str()));
}
