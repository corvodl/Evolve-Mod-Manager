//! Display-only character roster; game asset ownership is resolved separately.
pub struct Character {
    pub name: &'static str,
    pub group: &'static str,
    pub headshot: &'static str,
}

// Names and headshots follow modded-evolve.com/assets/js/images.js (IMAGES.heads).
// These URLs are presentation assets, never evidence that a game file belongs to a character.
pub const CHARACTERS: &[Character] = &[
    Character {
        name: "Goliath",
        group: "Monster",
        headshot: "thumb_mnstr_goliatha.png",
    },
    Character {
        name: "Meteor Goliath",
        group: "Monster",
        headshot: "thumb_mnstr_goliathmeteor.png",
    },
    Character {
        name: "Kraken",
        group: "Monster",
        headshot: "thumb_mnstr_groundera.png",
    },
    Character {
        name: "Elder Kraken",
        group: "Monster",
        headshot: "thumb_mnstr_krakenelder.png",
    },
    Character {
        name: "Wraith",
        group: "Monster",
        headshot: "thumb_mnstr_teleportera.png",
    },
    Character {
        name: "Behemoth",
        group: "Monster",
        headshot: "thumb_mnstr_rocky.png",
    },
    Character {
        name: "Glacial Behemoth",
        group: "Monster",
        headshot: "thumb_mnstr_rockybobcicle.png",
    },
    Character {
        name: "Gorgon",
        group: "Monster",
        headshot: "thumb_mnstr_lurker.png",
    },
    Character {
        name: "Markov",
        group: "Assault",
        headshot: "thumb_markov.png",
    },
    Character {
        name: "Blitz Markov",
        group: "Assault",
        headshot: "thumb_markovblitz.png",
    },
    Character {
        name: "Hyde",
        group: "Assault",
        headshot: "thumb_hyde.png",
    },
    Character {
        name: "Parnell",
        group: "Assault",
        headshot: "thumb_parnell.png",
    },
    Character {
        name: "Torvald",
        group: "Assault",
        headshot: "thumb_torvald.png",
    },
    Character {
        name: "Lennox",
        group: "Assault",
        headshot: "thumb_lennox.png",
    },
    Character {
        name: "Val",
        group: "Medic",
        headshot: "thumb_val.png",
    },
    Character {
        name: "Rogue Val",
        group: "Medic",
        headshot: "thumb_valrogue.png",
    },
    Character {
        name: "Lazarus",
        group: "Medic",
        headshot: "thumb_lazarus.png",
    },
    Character {
        name: "Caira",
        group: "Medic",
        headshot: "thumb_caira.png",
    },
    Character {
        name: "Quantum Caira",
        group: "Medic",
        headshot: "thumb_cairalowgrav.png",
    },
    Character {
        name: "Slim",
        group: "Medic",
        headshot: "thumb_slim.png",
    },
    Character {
        name: "Emet",
        group: "Medic",
        headshot: "thumb_emet.png",
    },
    Character {
        name: "Paladin Parnell",
        group: "Medic",
        headshot: "thumb_parnelltank.png",
    },
    Character {
        name: "Hank",
        group: "Support",
        headshot: "thumb_hank.png",
    },
    Character {
        name: "Tech Sergeant Hank",
        group: "Support",
        headshot: "thumb_hanksgt.png",
    },
    Character {
        name: "Bucket",
        group: "Support",
        headshot: "thumb_bucket.png",
    },
    Character {
        name: "Cabot",
        group: "Support",
        headshot: "thumb_cabot.png",
    },
    Character {
        name: "Battle Cabot",
        group: "Support",
        headshot: "thumb_cabotbattle.png",
    },
    Character {
        name: "Sunny",
        group: "Support",
        headshot: "thumb_sunny.png",
    },
    Character {
        name: "Kala",
        group: "Support",
        headshot: "thumb_tech.png",
    },
    Character {
        name: "Maggie",
        group: "Trapper",
        headshot: "thumb_maggie.png",
    },
    Character {
        name: "Wasteland Maggie",
        group: "Trapper",
        headshot: "thumb_maggiewasteland.png",
    },
    Character {
        name: "Griffin",
        group: "Trapper",
        headshot: "thumb_griffin.png",
    },
    Character {
        name: "Electro Griffin",
        group: "Trapper",
        headshot: "thumb_griffinelectro.png",
    },
    Character {
        name: "Abe",
        group: "Trapper",
        headshot: "thumb_abe.png",
    },
    Character {
        name: "Renegade Abe",
        group: "Trapper",
        headshot: "thumb_aberenegade.png",
    },
    Character {
        name: "Crow",
        group: "Trapper",
        headshot: "thumb_crow.png",
    },
    Character {
        name: "Jack",
        group: "Trapper",
        headshot: "thumb_jackal.png",
    },
];

pub fn matches_search(name: &str, group: &str, search: &str) -> bool {
    name.to_ascii_lowercase()
        .contains(&search.to_ascii_lowercase())
        || group
            .to_ascii_lowercase()
            .contains(&search.to_ascii_lowercase())
}

pub fn load_headshots() -> iced::Task<super::Message> {
    iced::Task::perform(
        async {
            tokio::task::spawn_blocking(|| {
                use std::io::Read;
                let Ok(client) = reqwest::blocking::Client::builder()
                    .timeout(std::time::Duration::from_secs(4))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                else {
                    return Vec::new();
                };
                CHARACTERS
                    .iter()
                    .filter_map(|character| {
                        let url = format!(
                            "https://assets.modded-evolve.com/images/thumbnails/{}",
                            character.headshot
                        );
                        let response = client.get(url).send().ok()?.error_for_status().ok()?;
                        if response
                            .content_length()
                            .is_some_and(|size| size > 512 * 1024)
                        {
                            return None;
                        }
                        let mut bytes = Vec::new();
                        response.take(512 * 1024 + 1).read_to_end(&mut bytes).ok()?;
                        if bytes.len() > 512 * 1024 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                            return None;
                        }
                        let (width, height) = image::ImageReader::with_format(
                            std::io::Cursor::new(&bytes),
                            image::ImageFormat::Png,
                        )
                        .into_dimensions()
                        .ok()?;
                        (width > 0 && height > 0 && width <= 1024 && height <= 1024)
                            .then_some((character.name, bytes))
                    })
                    .collect()
            })
            .await
            .unwrap_or_default()
        },
        super::Message::ModdingHeadshotsLoaded,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_distinct_roster_and_search() {
        assert_eq!(CHARACTERS.len(), 37);
        let mut names = std::collections::HashSet::new();
        for character in CHARACTERS {
            assert!(names.insert(character.name));
            assert!(!character.headshot.contains('/'));
        }
        assert!(matches_search("Meteor Goliath", "Monster", "gOlIaTh"));
        assert!(!matches_search("Goliath", "Monster", "val"));
    }
}
