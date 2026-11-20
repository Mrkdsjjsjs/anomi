//! Discord's list of games as Settings' picker searches it, read from the
//! compact list detection keeps (`catalog.rs`) as the picker opens and
//! dropped as it closes: each game's names one after another in one block
//! of text, the same lowercase in another, and the game's id beside where
//! they are. Discord's 25,000 games take about 1.5 MiB so; a search reads
//! the lowercase block and copies out only the games it gives.

use std::{
    io::{BufReader, Read},
    path::Path,
};

use anyhow::Result;
use serde::{
    Deserialize,
    de::{SeqAccess, Visitor},
};

/// Between a game's name and each of its other names, in both blocks.
const BETWEEN: char = '\u{1f}';

/// A game as the compact list keeps it; what else it holds is skipped.
#[derive(Deserialize)]
struct Listed {
    #[serde(default)]
    id: Option<String>,
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
}

/// Where a game's names are: its id, and the byte ranges of its names in
/// the two blocks.
struct Entry {
    id: u64,
    text: (u32, u32),
    folded: (u32, u32),
}

/// Discord's games, searchable by name and by their other names.
pub struct Directory {
    /// Each game's name, then its other names, as Discord writes them.
    text: String,
    /// The same, lowercase, which a search reads.
    folded: String,
    games: Vec<Entry>,
    /// How many games have other names: none in a list kept before
    /// Yapfast kept them.
    aliased: usize,
}

/// A game a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its application on Discord.
    pub id: u64,
    pub name: String,
    /// The other name it was found by, when its own name didn't match.
    pub alias: Option<String>,
}

/// What a search found: the best of the games that matched, as many as
/// were asked for at most.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Search {
    pub games: Vec<Found>,
    /// How many games matched in all.
    pub matched: usize,
}

/// How well a name matched, best first: at its start, at a word's, inside
/// a word. A game's own name ranks above its other names at each.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    Start,
    Word,
    Inside,
}

impl Directory {
    /// Reads the list at `path`.
    pub fn read(path: &Path) -> Result<Self> {
        Self::from_reader(super::catalog::open(path)?)
    }

    /// Reads a list in Discord's shape, one game at a time.
    pub fn from_reader(reader: impl Read) -> Result<Self> {
        struct Games<'a>(&'a mut Directory);
        impl<'de> Visitor<'de> for Games<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a list of games")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
                while let Some(game) = seq.next_element::<Listed>()? {
                    self.0.add(game);
                }
                Ok(())
            }
        }
        use serde::Deserializer as _;
        let mut directory = Directory {
            text: String::new(),
            folded: String::new(),
            games: Vec::new(),
            aliased: 0,
        };
        let mut decoder = serde_json::Deserializer::from_reader(BufReader::new(reader));
        (&mut decoder).deserialize_seq(Games(&mut directory))?;
        decoder.end()?;
        directory.text.shrink_to_fit();
        directory.folded.shrink_to_fit();
        directory.games.shrink_to_fit();
        Ok(directory)
    }

    /// Adds a game Discord names by id; its other names that only differ
    /// from its name, or each other, in case are left out.
    fn add(&mut self, game: Listed) {
        let Some(id) = (game.id.as_deref())
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|&id| id > 0)
        else {
            return;
        };
        let name = plain(&game.name);
        if name.is_empty() {
            return;
        }
        let mut folded: Vec<String> = vec![name.to_lowercase()];
        let mut names = vec![name];
        for alias in &game.aliases {
            let alias = plain(alias);
            let lower = alias.to_lowercase();
            if !alias.is_empty() && !folded.contains(&lower) {
                folded.push(lower);
                names.push(alias);
            }
        }
        self.aliased += usize::from(names.len() > 1);
        let (text_start, folded_start) = (self.text.len(), self.folded.len());
        for (at, (name, lower)) in names.iter().zip(&folded).enumerate() {
            if at > 0 {
                self.text.push(BETWEEN);
                self.folded.push(BETWEEN);
            }
            self.text.push_str(name);
            self.folded.push_str(lower);
        }
        let span = |start: usize, end: usize| (start as u32, end as u32);
        self.games.push(Entry {
            id,
            text: span(text_start, self.text.len()),
            folded: span(folded_start, self.folded.len()),
        });
    }

    pub fn len(&self) -> usize {
        self.games.len()
    }

    pub fn is_empty(&self) -> bool {
        self.games.is_empty()
    }

    /// The list was kept before Yapfast kept games' other names; see
    /// `Catalog::old_shape`.
    pub fn old_shape(&self) -> bool {
        !self.games.is_empty() && self.aliased == 0
    }

    /// The games whose name or another name holds `query`, whatever its
    /// case: at most `limit`, those matching at the start first, then at a
    /// word's start, then inside a word, each the shortest name first, as
    /// Discord lists them among equals. An empty query gives the first
    /// games of the list, Discord's oldest and best known.
    pub fn search(&self, query: &str, limit: usize) -> Search {
        let query = query.trim().to_lowercase();
        if query.is_empty() {
            let games = (self.games.iter().take(limit))
                .map(|entry| self.found(entry, 0))
                .collect();
            return Search {
                games,
                matched: self.games.len(),
            };
        }
        // (how well, how long its name, which game, by which name)
        let mut hits: Vec<(u8, u32, u32, u8)> = Vec::new();
        for (index, entry) in self.games.iter().enumerate() {
            let names = &self.folded[entry.folded.0 as usize..entry.folded.1 as usize];
            if let Some((rank, which)) = best(names, &query) {
                let length = names.split(BETWEEN).next().map_or(0, str::len) as u32;
                hits.push((rank, length, index as u32, which));
            }
        }
        let matched = hits.len();
        if hits.len() > limit {
            // The best `limit` before the rest, in no order yet.
            let _ = hits.select_nth_unstable(limit);
            hits.truncate(limit);
        }
        hits.sort_unstable();
        let games = (hits.into_iter())
            .map(|(_, _, index, which)| self.found(&self.games[index as usize], which))
            .collect();
        Search { games, matched }
    }

    /// The game at `entry`, found by its name (`which` 0) or another.
    fn found(&self, entry: &Entry, which: u8) -> Found {
        let mut names = self.text[entry.text.0 as usize..entry.text.1 as usize].split(BETWEEN);
        let name = names.next().unwrap_or_default().to_owned();
        let alias = (which > 0)
            .then(|| names.nth(usize::from(which) - 1))
            .flatten()
            .map(str::to_owned);
        Found {
            id: entry.id,
            name,
            alias,
        }
    }
}

/// How well the best of a game's lowercase `names` holds `query`, and
/// which name it is (0 for its own); none if none does.
fn best(names: &str, query: &str) -> Option<(u8, u8)> {
    (names.split(BETWEEN).enumerate())
        .filter_map(|(which, name)| {
            let place = place(name, query)?;
            let rank = place as u8 * 2 + u8::from(which > 0);
            Some((rank, which.min(u8::MAX as usize) as u8))
        })
        .min()
}

/// Where `query` is in `name`, at best.
fn place(name: &str, query: &str) -> Option<Place> {
    let mut found = None;
    for (at, _) in name.match_indices(query) {
        if at == 0 {
            return Some(Place::Start);
        }
        let after_word = name[..at]
            .chars()
            .next_back()
            .is_some_and(|before| !before.is_alphanumeric());
        if after_word {
            return Some(Place::Word);
        }
        found = Some(Place::Inside);
    }
    found
}

/// A name without its surrounding spaces, its control characters as spaces.
fn plain(name: &str) -> String {
    name.trim()
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn directory(games: serde_json::Value) -> Directory {
        Directory::from_reader(serde_json::to_vec(&games).unwrap().as_slice()).unwrap()
    }

    fn names(search: &Search) -> Vec<&str> {
        search.games.iter().map(|game| game.name.as_str()).collect()
    }

    #[test]
    fn a_search_gives_at_most_the_limit_and_counts_the_rest() {
        let games: Vec<_> = (1..=50)
            .map(|n| json!({"id": (100 + n).to_string(), "name": format!("Game {n}")}))
            .collect();
        let directory = directory(json!(games));
        let found = directory.search("game", 10);
        assert_eq!(found.games.len(), 10);
        assert_eq!(found.matched, 50);
        assert_eq!(found.games[0].name, "Game 1", "the shortest names first");
        let first = directory.search("", 5);
        assert_eq!(
            names(&first),
            ["Game 1", "Game 2", "Game 3", "Game 4", "Game 5"]
        );
        assert_eq!(first.matched, 50);
        assert!(directory.search("nothing like it", 10).games.is_empty());
    }

    #[test]
    fn other_names_find_a_game_and_say_which() {
        let directory = directory(json!([
            {"id":"1", "name":"Rainbow Six Siege",
                "aliases":["Tom Clancy's Rainbow Six Siege", "rainbow six siege"]},
            {"id":"2", "name":"Rocket League", "aliases":["Rocket League"]},
        ]));
        let found = directory.search("tom clancy", 5);
        assert_eq!(found.games.len(), 1);
        assert_eq!(found.games[0].id, 1);
        assert_eq!(
            found.games[0].alias.as_deref(),
            Some("Tom Clancy's Rainbow Six Siege")
        );
        // Its own name matching, no other name is said.
        assert_eq!(directory.search("rainbow", 5).games[0].alias, None);
        // An other name that is the name again doesn't count as one.
        assert!(!directory.old_shape());
        assert_eq!(directory.aliased, 1);
    }

    #[test]
    fn case_does_not_matter_and_better_places_come_first() {
        let directory = directory(json!([
            {"id":"1", "name":"LEGO Minecraft Adventures"},
            {"id":"2", "name":"Minecraft Dungeons"},
            {"id":"3", "name":"Minecraft"},
            {"id":"4", "name":"Spacecraft", "aliases":["Minecraft Space"]},
            {"id":"5", "name":"Über Straße"},
        ]));
        let found = directory.search("MINECRAFT", 10);
        assert_eq!(
            names(&found),
            [
                "Minecraft",
                "Minecraft Dungeons",
                "Spacecraft",
                "LEGO Minecraft Adventures"
            ]
        );
        // Inside a word everywhere, the shortest name first; its own name
        // ranks a game above its other names.
        assert_eq!(
            names(&directory.search("craft", 10)),
            [
                "Minecraft",
                "Spacecraft",
                "Minecraft Dungeons",
                "LEGO Minecraft Adventures"
            ]
        );
        assert_eq!(
            names(&directory.search("über STRASSE", 10)),
            Vec::<&str>::new()
        );
        assert_eq!(names(&directory.search("ÜBER straße", 10)), ["Über Straße"]);
    }

    #[test]
    fn games_without_an_id_are_not_discords() {
        let directory = directory(json!([
            {"name":"Nobody's"},
            {"id":"0", "name":"Zero"},
            {"id":"7", "name":"  Spaced\u{1}Out  "},
        ]));
        assert_eq!(directory.len(), 1);
        assert_eq!(names(&directory.search("spaced out", 5)), ["Spaced Out"]);
        assert!(directory.old_shape(), "no game has another name");
    }
}
