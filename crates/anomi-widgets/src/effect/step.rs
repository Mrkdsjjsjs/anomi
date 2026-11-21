//! What the effect does at each frame: its play moved on, the players of
//! the layers that show made and those of the layers that hid dropped, its
//! still kept while it rests, the next frame it asks for, and whether a
//! layer's picture is wanted.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use iced::{Rectangle, Vector};

use anomi_theme::{Motion, Renderer};

use super::playback::{Course, DELAY, Look, shown};
use super::{EffectLayer, Play, ProfileEffect, State, offset, size_of};
use crate::picture::Player;

impl<Message> ProfileEffect<'_, Message> {
    /// Whether to say now that a layer's picture is wanted, which it says
    /// once a play: it waits for one as a card opens, or one shows without
    /// it.
    pub(super) fn wants(&self, state: &mut State) -> bool {
        let missing = if state.playback.pending() {
            true
        } else if state.playback.playing() {
            let at = state.playback.at();
            let missing = |layer: &EffectLayer| layer.picture.is_none() && shown(layer, at);
            self.effect.layers.iter().any(missing)
        } else {
            state.wanted = false;
            return false;
        };
        let wants = missing && !state.wanted;
        state.wanted |= missing;
        wants
    }

    /// Takes the frame at `now`: moves the play on, starts the players of
    /// the layers that came to show and drops those of the layers that
    /// hid, and says when the next frame is due.
    pub(super) fn frame(
        &self,
        state: &mut State,
        bounds: Rectangle,
        renderer: &Renderer,
        viewport: &Rectangle,
        now: Instant,
    ) -> Option<Instant> {
        let layers = &self.effect.layers;
        let timing = timing(layers);
        if state.timing.replace(timing).is_some_and(|was| was != timing) {
            // Another effect: its play starts over, as a change of what's
            // asked does.
            state.playback.restart();
            state.layers.clear();
            state.wanted = false;
        }
        let course = self.course();
        let asked = self.asked(state.hovered);
        // As its layers' players: in front or visible under Everything,
        // animations on and the window in use.
        let may = self.motion.watched() && !layers.is_empty();
        let playing = state.playback.frame(asked, may, self.ready(), course, now);
        let at = state.playback.at();
        let mut due = state.playback.next(layers, course, now);
        let mut sooner = |next: Option<Instant>| due = due.into_iter().chain(next).min();
        state.layers.resize_with(layers.len(), || None);
        for (layer, slot) in layers.iter().zip(&mut state.layers) {
            let picture = layer
                .picture
                .as_ref()
                .filter(|_| playing && shown(layer, at));
            let Some(picture) = picture else {
                *slot = None;
                continue;
            };
            let player = slot.get_or_insert_with(Player::from_start);
            if let Some((drawn, _)) = self.place(bounds, offset(layer), size_of(renderer, picture))
            {
                sooner(player.update(picture, drawn, viewport, self.motion, now));
            }
        }
        if state.playback.look() == Look::Rest
            && !self.effect.intermittent
            && let Some(still) = &self.effect.still
            && let Some((drawn, _)) = self.place(bounds, Vector::ZERO, size_of(renderer, still))
        {
            // A still, as Slint's `Image`: it never moves.
            sooner(
                state
                    .still
                    .update(still, drawn, viewport, Motion::default(), now),
            );
        }
        due
    }

    /// What its play runs through: its layers' end, whether they loop,
    /// and, as a card opens, the delay before them.
    fn course(&self) -> Course {
        Course {
            end: self.effect.end(),
            loops: self.effect.loops(),
            delay: if self.play == Play::Open {
                DELAY
            } else {
                Duration::ZERO
            },
        }
    }

    /// Whether its layers may start: as a card opens, once every layer's
    /// picture has come; otherwise at once, each layer as its picture
    /// does, which the app asks for as it's wanted.
    fn ready(&self) -> bool {
        let came = |layer: &EffectLayer| layer.picture.is_some();
        self.play != Play::Open || self.effect.layers.iter().all(came)
    }
}

/// The layers' count and timings as one number: another effect's differ,
/// while its pictures coming leave it as it was.
fn timing(layers: &[EffectLayer]) -> u64 {
    let mut hasher = DefaultHasher::new();
    layers.len().hash(&mut hasher);
    for layer in layers {
        let place = (layer.x.to_bits(), layer.y.to_bits());
        (layer.start, layer.duration, layer.looped, layer.loop_delay, place).hash(&mut hasher);
    }
    hasher.finish()
}
