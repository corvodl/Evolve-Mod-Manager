//! Display-only character roster; game asset ownership is resolved separately.
pub struct Character {
    pub name: &'static str,
    pub group: &'static str,
    /// Full-body cut-out, 180x240 with the feet on the bottom edge.
    pub portrait: &'static [u8],
}

/// Classes in the order the roster shows them.
pub const GROUPS: [&str; 5] = ["Monster", "Assault", "Medic", "Support", "Trapper"];

// Names follow modded-evolve.com; portraits are its full-body images, made
// small by scripts/build-roster-portraits.py. They are presentation only,
// never evidence that a game file belongs to a character.
pub const CHARACTERS: &[Character] = &[
    Character {
        name: "Goliath",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/goliath.png"),
    },
    Character {
        name: "Meteor Goliath",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/goliathmeteor.png"),
    },
    Character {
        name: "Kraken",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/kraken.png"),
    },
    Character {
        name: "Elder Kraken",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/krakenelder.png"),
    },
    Character {
        name: "Wraith",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/wraith.png"),
    },
    Character {
        name: "Behemoth",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/behemoth.png"),
    },
    Character {
        name: "Glacial Behemoth",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/rockybobcicle.png"),
    },
    Character {
        name: "Gorgon",
        group: "Monster",
        portrait: include_bytes!("../../../assets/roster/gorgon.png"),
    },
    Character {
        name: "Markov",
        group: "Assault",
        portrait: include_bytes!("../../../assets/roster/markov.png"),
    },
    Character {
        name: "Blitz Markov",
        group: "Assault",
        portrait: include_bytes!("../../../assets/roster/markovblitz.png"),
    },
    Character {
        name: "Hyde",
        group: "Assault",
        portrait: include_bytes!("../../../assets/roster/hyde.png"),
    },
    Character {
        name: "Parnell",
        group: "Assault",
        portrait: include_bytes!("../../../assets/roster/parnell.png"),
    },
    Character {
        name: "Torvald",
        group: "Assault",
        portrait: include_bytes!("../../../assets/roster/torvald.png"),
    },
    Character {
        name: "Lennox",
        group: "Assault",
        portrait: include_bytes!("../../../assets/roster/lennox.png"),
    },
    Character {
        name: "Val",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/val.png"),
    },
    Character {
        name: "Rogue Val",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/valrogue.png"),
    },
    Character {
        name: "Lazarus",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/lazarus.png"),
    },
    Character {
        name: "Caira",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/caira.png"),
    },
    Character {
        name: "Quantum Caira",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/cairaquantum.png"),
    },
    Character {
        name: "Slim",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/slim.png"),
    },
    Character {
        name: "Emet",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/emet.png"),
    },
    Character {
        name: "Paladin Parnell",
        group: "Medic",
        portrait: include_bytes!("../../../assets/roster/parnelltank.png"),
    },
    Character {
        name: "Hank",
        group: "Support",
        portrait: include_bytes!("../../../assets/roster/hank.png"),
    },
    Character {
        name: "Tech Sergeant Hank",
        group: "Support",
        portrait: include_bytes!("../../../assets/roster/hanksgt.png"),
    },
    Character {
        name: "Bucket",
        group: "Support",
        portrait: include_bytes!("../../../assets/roster/bucket.png"),
    },
    Character {
        name: "Cabot",
        group: "Support",
        portrait: include_bytes!("../../../assets/roster/cabot.png"),
    },
    Character {
        name: "Battle Cabot",
        group: "Support",
        portrait: include_bytes!("../../../assets/roster/cabotbattle.png"),
    },
    Character {
        name: "Sunny",
        group: "Support",
        portrait: include_bytes!("../../../assets/roster/sunny.png"),
    },
    Character {
        name: "Kala",
        group: "Support",
        portrait: include_bytes!("../../../assets/roster/kala.png"),
    },
    Character {
        name: "Maggie",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/maggiedaisy.png"),
    },
    Character {
        name: "Wasteland Maggie",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/maggiewastelanddaisy.png"),
    },
    Character {
        name: "Griffin",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/griffin.png"),
    },
    Character {
        name: "Electro Griffin",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/griffinelectro.png"),
    },
    Character {
        name: "Abe",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/abe.png"),
    },
    Character {
        name: "Renegade Abe",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/aberenegade.png"),
    },
    Character {
        name: "Crow",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/crow.png"),
    },
    Character {
        name: "Jack",
        group: "Trapper",
        portrait: include_bytes!("../../../assets/roster/jack.png"),
    },
];

pub fn matches_search(name: &str, group: &str, search: &str) -> bool {
    name.to_ascii_lowercase()
        .contains(&search.to_ascii_lowercase())
        || group
            .to_ascii_lowercase()
            .contains(&search.to_ascii_lowercase())
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
            assert!(character.portrait.starts_with(b"\x89PNG"));
            assert!(GROUPS.contains(&character.group));
        }
        assert!(matches_search("Meteor Goliath", "Monster", "gOlIaTh"));
        assert!(!matches_search("Goliath", "Monster", "val"));
    }
}
