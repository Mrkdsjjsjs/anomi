//! The card beside someone's row under a voice channel in the channel list,
//! as Discord's client shows it while the pointer rests there: the game they
//! play and where it's sold, their stream and its preview. The interface
//! says when it shows and when it goes ([`Action::HoverVoicePerson`]);
//! nothing is asked for while none shows.

use super::*;
use crate::chat::StorePage;

impl Application {
    /// The card of `user` in the voice channel `room` shows, or with none,
    /// it went: their stream's preview counts as on screen
    /// (`stream_previews`), and where their game is sold is asked for once,
    /// unless it's the game the last card asked about.
    pub(super) fn hover_voice_person(
        &mut self,
        hovered: Option<(String, String)>,
        now: u64,
    ) -> Vec<Effect> {
        let application = hovered.as_ref().and_then(|(room, user)| {
            let listed = self.state.rooms.iter().find(|listed| listed.id == *room)?;
            let person = listed.people.iter().find(|person| person.id == *user)?;
            let application = person.playing()?.application?;
            application
                .bytes()
                .all(|byte| byte.is_ascii_digit())
                .then(|| application.to_owned())
        });
        let mut effects = self.hover_stream(hovered, now);
        if let Some(application) = application
            && (self.state.game_store.as_ref()).is_none_or(|(known, _)| *known != application)
        {
            self.state.game_store = Some((application.clone(), None));
            effects.push(Effect::Backend(Command::Discord(
                DiscordCommand::GameStore { application },
            )));
        }
        effects
    }

    /// Where the game a card asked about is sold, kept while it's still the
    /// last asked about.
    pub(super) fn keep_game_store(&mut self, application: String, page: Option<StorePage>) {
        if (self.state.game_store.as_ref()).is_some_and(|(known, _)| *known == application) {
            self.state.game_store = Some((application, page));
        }
    }
}

#[cfg(all(test, feature = "discord"))]
mod tests {
    use super::*;
    use crate::chat::VoicePerson;

    fn lounge_with(people: Vec<VoicePerson>) -> (Application, String) {
        let mut app = Application::default();
        let lounge = crate::discord::room_id("10", "11");
        app.state.rooms = vec![Arc::new(ChatRoom {
            id: lounge.clone(),
            voice: true,
            people,
            ..preview::preview_room("lounge", "Lounge", false, 3)
        })];
        (app, lounge)
    }

    fn playing(id: &str, streaming: bool) -> VoicePerson {
        let activities = yapfast_discord::activity::activities(&serde_json::json!([
            {"type": 0, "name": "Dota 2", "application_id": "570", "timestamps": {"start": 1000}},
        ]));
        VoicePerson {
            id: id.into(),
            name: id.into(),
            streaming,
            activities,
            ..Default::default()
        }
    }

    fn asked(effects: &[Effect]) -> (Vec<&str>, Vec<&str>) {
        let mut previews = Vec::new();
        let mut stores = Vec::new();
        for effect in effects {
            match effect {
                Effect::Backend(Command::Discord(DiscordCommand::StreamPreview {
                    user, ..
                })) => {
                    previews.push(user.as_str());
                }
                Effect::Backend(Command::Discord(DiscordCommand::GameStore { application })) => {
                    stores.push(application.as_str());
                }
                _ => {}
            }
        }
        (previews, stores)
    }

    #[test]
    fn a_streamers_card_asks_for_the_preview_and_the_store_once_and_lets_go_as_it_goes() {
        let (mut app, lounge) = lounge_with(vec![playing("2", true)]);
        let bob = Some((lounge.clone(), "2".to_owned()));
        let shown = app.dispatch(Action::HoverVoicePerson(bob.clone()));
        assert_eq!(asked(&shown), (vec!["2"], vec!["570"]));
        // Its card shows a while: due as often as a tile's, outside a call.
        let due = app
            .stream_previews_due()
            .expect("the card's preview falls due");
        assert_eq!(asked(&app.ask_due_previews(due)), (vec!["2"], vec![]));
        app.apply(Event::StreamPreview {
            room: lounge.clone(),
            user: "2".into(),
            picture: Some(Arc::new(Picture::default())),
        });
        assert!(app.state.stream_previews.contains_key("2"));
        app.apply(Event::GameStore {
            application: "570".into(),
            page: StorePage::of("steam", "570"),
        });
        let page = app
            .state
            .game_store
            .as_ref()
            .and_then(|(_, page)| page.as_ref());
        assert_eq!(
            page.map(|page| page.url.as_str()),
            Some("https://store.steampowered.com/app/570/")
        );
        // The card goes: its preview with it, and nothing falls due.
        assert_eq!(
            asked(&app.dispatch(Action::HoverVoicePerson(None))),
            (vec![], vec![])
        );
        assert!(app.state.stream_previews.is_empty());
        assert_eq!(app.stream_previews_due(), None);
        // Back on him: the preview is asked anew, the store isn't.
        assert_eq!(
            asked(&app.dispatch(Action::HoverVoicePerson(bob))),
            (vec!["2"], vec![])
        );
    }

    #[test]
    fn a_players_card_asks_for_no_preview_and_a_late_store_isnt_kept() {
        let (mut app, lounge) = lounge_with(vec![playing("2", false), playing("3", false)]);
        let card = |user: &str| Action::HoverVoicePerson(Some((lounge.clone(), user.to_owned())));
        assert_eq!(asked(&app.dispatch(card("2"))), (vec![], vec!["570"]));
        assert_eq!(
            app.stream_previews_due(),
            None,
            "nothing to show but the game"
        );
        // The same game on the next card asks nothing more.
        assert_eq!(asked(&app.dispatch(card("3"))), (vec![], vec![]));
        app.state.game_store = Some(("42".into(), None));
        app.apply(Event::GameStore {
            application: "570".into(),
            page: StorePage::of("steam", "570"),
        });
        assert_eq!(
            app.state.game_store,
            Some(("42".into(), None)),
            "another game was asked about since"
        );
    }
}
