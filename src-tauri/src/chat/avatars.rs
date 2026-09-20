//! Conversation faces. Ids match `src/lib/avatars.ts`.

use std::collections::HashSet;

pub struct Avatar {
    pub id: &'static str,
    pub name: &'static str,
}

/// Spoken name is the food, not the filename.
pub const AVATARS: &[Avatar] = &[
    Avatar {
        id: "apple",
        name: "Apple",
    },
    Avatar {
        id: "artichoke",
        name: "Artichoke",
    },
    Avatar {
        id: "bread",
        name: "Bread",
    },
    Avatar {
        id: "cheese",
        name: "Cheese",
    },
    Avatar {
        id: "eggplant",
        name: "Eggplant",
    },
    Avatar {
        id: "fig",
        name: "Fig",
    },
    Avatar {
        id: "garlic",
        name: "Garlic",
    },
    Avatar {
        id: "grape",
        name: "Grape",
    },
    Avatar {
        id: "hazelnut",
        name: "Hazelnut",
    },
    Avatar {
        id: "honey",
        name: "Honey",
    },
    Avatar {
        id: "lemon",
        name: "Lemon",
    },
    Avatar {
        id: "mushroom",
        name: "Mushroom",
    },
    Avatar {
        id: "onion",
        name: "Onion",
    },
    Avatar {
        id: "orange",
        name: "Orange",
    },
    Avatar {
        id: "peach",
        name: "Peach",
    },
    Avatar {
        id: "pear",
        name: "Pear",
    },
    Avatar {
        id: "pepper",
        name: "Pepper",
    },
    Avatar {
        id: "pomegranate",
        name: "Pomegranate",
    },
    Avatar {
        id: "potato",
        name: "Potato",
    },
    Avatar {
        id: "pumpkin",
        name: "Pumpkin",
    },
    Avatar {
        id: "quince",
        name: "Quince",
    },
    Avatar {
        id: "tomato",
        name: "Tomato",
    },
    Avatar {
        id: "walnut",
        name: "Walnut",
    },
];

pub fn name_for(id: &str) -> Option<&'static str> {
    AVATARS
        .iter()
        .find(|avatar| avatar.id == id)
        .map(|a| a.name)
}

pub fn pick_id(thread_id: &str, used: &HashSet<String>) -> &'static str {
    let unused: Vec<&Avatar> = AVATARS
        .iter()
        .filter(|avatar| !used.contains(avatar.id))
        .collect();
    if unused.is_empty() {
        return AVATARS[hash_thread_id(thread_id) % AVATARS.len()].id;
    }
    unused[hash_thread_id(thread_id) % unused.len()].id
}

fn hash_thread_id(thread_id: &str) -> usize {
    let mut hash: u32 = 0;
    for byte in thread_id.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(u32::from(byte));
    }
    hash as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_catalog_ids() {
        assert_eq!(name_for("tomato"), Some("Tomato"));
        assert_eq!(name_for("pomegranate"), Some("Pomegranate"));
        assert_eq!(name_for("nope"), None);
    }

    #[test]
    fn pick_skips_faces_already_in_use() {
        let used: HashSet<String> = AVATARS
            .iter()
            .skip(1)
            .map(|avatar| avatar.id.to_string())
            .collect();
        assert_eq!(pick_id("t_anything", &used), AVATARS[0].id);
    }

    #[test]
    fn pick_is_stable_when_every_face_is_taken() {
        let used: HashSet<String> = AVATARS.iter().map(|avatar| avatar.id.to_string()).collect();
        assert_eq!(pick_id("same-thread", &used), pick_id("same-thread", &used));
        assert!(name_for(pick_id("same-thread", &used)).is_some());
    }
}
