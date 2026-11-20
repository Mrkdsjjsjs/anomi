//! Which of the call's tiles is which, which one shows large, and what the
//! corner shows while the call is off screen: `Stage`, `Focus` and `Pip` of
//! `src/ui/tiles.rs`. The tiles' order is Discord's: everyone in the call's
//! order, and right before each one who shares a screen, their stream's
//! tile; this account's screen ("Your screen") right before its own. A
//! stream is always a tile of its own, whether its streamer films or not.

use yapfast_core::voice::Participant;

/// The tile shown large, as a click on it chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Focus {
    /// Someone's own tile, by participant id: their camera, or their
    /// picture.
    Person(String),
    /// Someone's stream, in its own tile, by participant id.
    Stream(String),
    /// This account's shared screen.
    OwnScreen,
}

/// What the corner shows while the call is off screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pip {
    None,
    Watched,
    Shared,
    /// The camera of the participant at this index.
    Camera(usize),
}

/// One of the call's tiles, by what it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// This account's shared screen.
    OwnScreen,
    /// The stream of the participant at this index.
    Stream(usize),
    /// The own tile of the participant at this index.
    Person(usize),
}

/// The call's tiles in order (see the module's doc).
#[derive(Clone, Copy)]
pub struct Stage<'a> {
    pub participants: &'a [Participant],
    pub sharing: bool,
}

impl Stage<'_> {
    /// The tiles in order. This account's screen leads while it isn't in
    /// the list yet, as while it joins.
    pub fn slots(&self) -> impl Iterator<Item = Slot> + '_ {
        let unlisted = !self.participants.iter().any(|person| person.local);
        let lead = (self.sharing && unlisted).then_some(Slot::OwnScreen);
        let each = self
            .participants
            .iter()
            .enumerate()
            .flat_map(move |(index, person)| {
                let stream = if person.local {
                    self.sharing.then_some(Slot::OwnScreen)
                } else {
                    person.streaming.then_some(Slot::Stream(index))
                };
                stream.into_iter().chain([Slot::Person(index)])
            });
        lead.into_iter().chain(each)
    }

    /// How many tiles there are.
    pub fn count(&self) -> usize {
        self.slots().count()
    }

    fn index(&self, id: &str) -> Option<usize> {
        self.participants.iter().position(|person| person.id == id)
    }

    /// The tile `focus` names, if it's in the call.
    fn named(&self, focus: &Focus) -> Option<Slot> {
        match focus {
            Focus::OwnScreen => self.sharing.then_some(Slot::OwnScreen),
            Focus::Person(id) => Some(Slot::Person(self.index(id)?)),
            Focus::Stream(id) => {
                let index = self.index(id)?;
                let them = &self.participants[index];
                (them.streaming && !them.local).then_some(Slot::Stream(index))
            }
        }
    }

    /// The slot of the tile `focus` shows, if it's in the call.
    pub fn slot(&self, focus: &Focus) -> Option<usize> {
        let named = self.named(focus)?;
        self.slots().position(|slot| slot == named)
    }

    /// The slot of the own tile of the participant at `person`.
    pub fn person_slot(&self, person: usize) -> Option<usize> {
        self.slots().position(|slot| slot == Slot::Person(person))
    }

    /// The slot of the stream of the participant at `person`, if they
    /// share one: its own tile.
    pub fn stream_slot(&self, person: usize) -> Option<usize> {
        let them = self.participants.get(person)?;
        let stream = if them.local {
            Slot::OwnScreen
        } else {
            Slot::Stream(person)
        };
        self.slots().position(|slot| slot == stream)
    }

    /// What a click on the tile at `slot` shows large.
    pub fn focus_at(&self, slot: usize) -> Option<Focus> {
        let id = |index: usize| self.participants[index].id.clone();
        Some(match self.slots().nth(slot)? {
            Slot::OwnScreen => Focus::OwnScreen,
            Slot::Stream(index) => Focus::Stream(id(index)),
            Slot::Person(index) => Focus::Person(id(index)),
        })
    }

    /// What the corner shows: what's large, if it moves; else the stream
    /// watched or shared; else the first camera of someone else.
    pub fn pip(&self, focus: Option<&Focus>, watching: Option<&str>) -> Pip {
        match focus {
            Some(Focus::OwnScreen) if self.sharing => return Pip::Shared,
            Some(Focus::Stream(id)) if watching == Some(id.as_str()) => return Pip::Watched,
            Some(Focus::Person(id)) => {
                if let Some(person) = self.index(id)
                    && self.participants[person].video.is_some()
                {
                    return Pip::Camera(person);
                }
            }
            _ => {}
        }
        if watching.is_some() {
            Pip::Watched
        } else if self.sharing {
            Pip::Shared
        } else {
            self.participants
                .iter()
                .position(|person| !person.local && person.video.is_some())
                .map_or(Pip::None, Pip::Camera)
        }
    }

    /// The slot of the tile the corner shows, which a click on it shows
    /// large.
    pub fn pip_slot(&self, pip: Pip, watching: Option<&str>) -> Option<usize> {
        match pip {
            Pip::None => None,
            Pip::Watched => self.slot(&Focus::Stream(watching?.to_owned())),
            Pip::Shared => self.slot(&Focus::OwnScreen),
            Pip::Camera(person) => self.person_slot(person),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(id: &str, streaming: bool, filming: bool) -> Participant {
        Participant {
            id: id.into(),
            user: id.into(),
            person: id.into(),
            local: id == "me",
            muted: false,
            deafened: false,
            server_muted: false,
            server_deafened: false,
            speaking: false,
            connected: true,
            problem: None,
            streaming,
            video: filming.then(yapfast_core::picture::Feed::default),
            avatar: None,
        }
    }

    /// Me sharing my screen, Bob sharing his and filming, Carol sharing
    /// hers alone, Dave listening: each stream right before its streamer.
    #[test]
    fn a_stream_has_a_tile_of_its_own_right_before_its_streamers() {
        let people = [
            person("me", true, false),
            person("bob", true, true),
            person("carol", true, false),
            person("dave", false, false),
        ];
        let stage = Stage {
            participants: &people,
            sharing: true,
        };
        let slots: Vec<Slot> = stage.slots().collect();
        assert_eq!(
            slots,
            [
                Slot::OwnScreen,
                Slot::Person(0),
                Slot::Stream(1),
                Slot::Person(1),
                Slot::Stream(2),
                Slot::Person(2),
                Slot::Person(3),
            ]
        );
        assert_eq!(stage.count(), 7);
        assert_eq!(stage.slot(&Focus::OwnScreen), Some(0));
        assert_eq!(stage.slot(&Focus::Stream("bob".into())), Some(2));
        assert_eq!(stage.slot(&Focus::Person("bob".into())), Some(3));
        assert_eq!(stage.slot(&Focus::Stream("carol".into())), Some(4));
        assert_eq!(stage.slot(&Focus::Stream("dave".into())), None, "not live");
        assert_eq!(stage.focus_at(1), Some(Focus::Person("me".into())));
        assert_eq!(stage.focus_at(2), Some(Focus::Stream("bob".into())));
        assert_eq!(stage.focus_at(5), Some(Focus::Person("carol".into())));
        assert_eq!(stage.focus_at(7), None);
        assert_eq!(
            (stage.stream_slot(0), stage.stream_slot(2)),
            (Some(0), Some(4))
        );
        assert_eq!(
            (stage.person_slot(3), stage.stream_slot(3)),
            (Some(6), None)
        );
    }

    #[test]
    fn this_accounts_screen_leads_until_it_is_listed() {
        let people = [person("bob", true, false)];
        let stage = Stage {
            participants: &people,
            sharing: true,
        };
        let slots: Vec<Slot> = stage.slots().collect();
        assert_eq!(slots, [Slot::OwnScreen, Slot::Stream(0), Slot::Person(0)]);
        // Not sharing, this account has its own tile alone.
        let me = [person("me", true, false)];
        let quiet = Stage {
            participants: &me,
            sharing: false,
        };
        assert_eq!(quiet.slots().collect::<Vec<_>>(), [Slot::Person(0)]);
    }

    #[test]
    fn the_corner_shows_what_is_large_else_what_is_watched() {
        let people = [person("me", false, false), person("bob", true, true)];
        let stage = Stage {
            participants: &people,
            sharing: false,
        };
        let bob = Focus::Person("bob".into());
        assert_eq!(stage.pip(Some(&bob), Some("bob")), Pip::Camera(1));
        assert_eq!(stage.pip_slot(Pip::Camera(1), Some("bob")), Some(2));
        assert_eq!(stage.pip(None, Some("bob")), Pip::Watched);
        assert_eq!(stage.pip_slot(Pip::Watched, Some("bob")), Some(1));
        assert_eq!(stage.pip(None, None), Pip::Camera(1));
    }
}
