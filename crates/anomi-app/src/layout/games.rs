//! The games added from Discord's list on Game activity, as `layout.json`
//! keeps them under `discord_games`: Anomi's own key, written only once a
//! game is added, which the Slint app keeps through its catch-all. Each is
//! what detection takes (`custom_games::CustomGame`): the program, the
//! game's name and id, whether its activity is off, and since when one
//! without a program shows. Keys a newer version adds to a game stay with
//! it.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// A game added from Discord's list.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscordGame {
    /// Its application on Discord, as Discord writes ids.
    pub id: String,
    pub name: String,
    /// The program it's tied to, by its full path, as detection matches
    /// it; empty for a game shown without one.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub executable: String,
    /// Its activity is off: its program still counts as the game, which
    /// shows nothing.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// When one without a program was last turned on, in Unix seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// Every key of it this version doesn't know, kept as it came.
    #[serde(flatten)]
    pub other: Map<String, Value>,
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::super::Layout;
    use super::*;

    fn layout(file: Value) -> Layout {
        let Value::Object(values) = file else {
            panic!("a layout is an object")
        };
        Layout::from_values(std::path::PathBuf::new(), values)
    }

    fn bytes(layout: &Layout) -> Value {
        serde_json::from_slice(&layout.bytes().unwrap()).unwrap()
    }

    #[test]
    fn a_layout_without_them_writes_none() {
        let layout = layout(json!({"game_executables": [["game.exe", "A Game"]]}));
        assert!(layout.discord_games.is_empty());
        let written = bytes(&layout);
        assert!(
            written.get("discord_games").is_none(),
            "the Slint app's file as it was"
        );
        assert_eq!(written["game_executables"], json!([["game.exe", "A Game"]]));
    }

    #[test]
    fn games_are_kept_with_only_what_they_have() {
        let mut layout = layout(json!({"a_newer_setting": 1}));
        layout.edit(|saved| {
            saved.discord_games = vec![
                DiscordGame {
                    id: "1400000000000000900".into(),
                    name: "Factorio".into(),
                    executable: "/opt/factorio/bin/x64/factorio".into(),
                    ..DiscordGame::default()
                },
                DiscordGame {
                    id: "1400000000000000902".into(),
                    name: "Deep Dwarves".into(),
                    hidden: true,
                    since: Some(1_700_000_000),
                    ..DiscordGame::default()
                },
            ];
        });
        let written = bytes(&layout);
        assert_eq!(
            written["discord_games"],
            json!([
                {"id": "1400000000000000900", "name": "Factorio",
                    "executable": "/opt/factorio/bin/x64/factorio"},
                {"id": "1400000000000000902", "name": "Deep Dwarves",
                    "hidden": true, "since": 1_700_000_000},
            ])
        );
        assert_eq!(
            written["a_newer_setting"], 1,
            "a key this app doesn't know stays"
        );
        let Value::Object(values) = written else {
            unreachable!()
        };
        let again = Layout::from_values(std::path::PathBuf::new(), values);
        assert_eq!(
            again.discord_games, layout.discord_games,
            "read back as written"
        );
    }

    #[test]
    fn a_games_keys_this_version_doesnt_know_stay() {
        let game = json!({"id": "1", "name": "Game", "overlay": true});
        let layout = layout(json!({"discord_games": [game.clone()]}));
        assert_eq!(layout.discord_games[0].name, "Game");
        assert_eq!(bytes(&layout)["discord_games"], json!([game]));
    }

    #[test]
    fn games_that_dont_read_lose_nothing_else() {
        let layout = layout(json!({"discord_games": "not a list", "members_hidden": true}));
        assert!(layout.discord_games.is_empty() && layout.members_hidden);
        assert_eq!(
            bytes(&layout)["discord_games"],
            "not a list",
            "kept as it came"
        );
    }
}
