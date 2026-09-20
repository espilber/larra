//! Recipes: a library of saved prompts.
//!
//! A Recipe is a name plus a prompt. Clicking one starts a new conversation
//! with the composer pre-filled; the user completes the «placeholders»,
//! picks a Shelf if the ask needs those files, and sends. Retrieval, the
//! gate, and citations are ordinary Chat.
//!
//! Restore adds missing defaults; individual defaults can be reset separately.
//! Custom Recipes and edits persist in `recipes.json`.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::paths::Paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// Shipped with Larra (still deletable; restorable in one click).
    #[serde(default)]
    pub builtin: bool,
    /// This Recipe is meant to run with a Shelf. Missing on older files.
    #[serde(default)]
    pub needs_shelf: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecipesFile {
    recipes: Vec<Recipe>,
}

/// Shipped Recipes: id and whether the prompt is meant for a Shelf.
pub const DEFAULTS: &[(&str, bool)] = &[
    ("reply-to-client", false),
    ("one-page-brief", true),
    ("compare-documents", true),
    ("document-key-terms", true),
    ("minutes-actions", false),
    ("translate", false),
    ("policy-qa", true),
    ("campaign-ideas", false),
];

fn default_recipes() -> Vec<Recipe> {
    default_recipes_in(&rust_i18n::locale())
}

fn default_recipes_in(locale: &str) -> Vec<Recipe> {
    DEFAULTS
        .iter()
        .map(|(id, needs_shelf)| {
            let name_key = format!("defaults.recipes.{id}.name");
            let prompt_key = format!("defaults.recipes.{id}.prompt");
            Recipe {
                id: (*id).to_string(),
                name: rust_i18n::t!(&name_key, locale = locale).to_string(),
                prompt: rust_i18n::t!(&prompt_key, locale = locale).to_string(),
                builtin: true,
                needs_shelf: *needs_shelf,
            }
        })
        .collect()
}

fn read(paths: &Paths) -> Option<Vec<Recipe>> {
    crate::paths::read_json::<RecipesFile>(&paths.recipes_path()).map(|f| f.recipes)
}

fn write(paths: &Paths, recipes: &[Recipe]) -> Result<()> {
    let file = RecipesFile {
        recipes: recipes.to_vec(),
    };
    crate::paths::write_json(&paths.recipes_path(), &file)
}

/// All recipes; seeds the defaults on first use. Edits stay until Restore.
pub fn list(paths: &Paths) -> Vec<Recipe> {
    let transaction = crate::paths::metadata_lock(&paths.recipes_path());
    let _write = crate::core::mutex_lock(&transaction);
    list_unlocked(paths)
}

fn list_unlocked(paths: &Paths) -> Vec<Recipe> {
    match read(paths) {
        Some(mut recipes) => {
            let mut changed = false;
            for recipe in recipes.iter_mut() {
                if recipe.id == "contract-key-terms" {
                    recipe.id = "document-key-terms".into();
                    changed = true;
                }
            }
            if changed {
                if let Err(error) = write(paths, &recipes) {
                    log::error!("saving recipes: {error:#}");
                }
            }
            recipes
        }
        None => {
            let defaults = default_recipes();
            if let Err(error) = write(paths, &defaults) {
                log::error!("saving default recipes: {error:#}");
            }
            defaults
        }
    }
}

fn validated_recipe_fields(name: &str, prompt: &str) -> Result<(String, String)> {
    let name = name.trim();
    let prompt = prompt.trim();
    if name.is_empty() {
        return Err(anyhow!("{}", rust_i18n::t!("errors.recipeNeedsName")));
    }
    if prompt.is_empty() {
        return Err(anyhow!("{}", rust_i18n::t!("errors.recipeNeedsPrompt")));
    }
    if prompt.chars().count() > crate::limits::PROMPT_MAX_CHARS {
        return Err(anyhow!("{}", rust_i18n::t!("errors.recipeTooLong")));
    }
    Ok((name.to_string(), prompt.to_string()))
}

pub fn create(paths: &Paths, name: &str, prompt: &str) -> Result<Recipe> {
    let transaction = crate::paths::metadata_lock(&paths.recipes_path());
    let _write = crate::core::mutex_lock(&transaction);
    let (name, prompt) = validated_recipe_fields(name, prompt)?;
    let mut recipes = list_unlocked(paths);
    let recipe = Recipe {
        id: format!("r_{}", uuid::Uuid::new_v4().simple()),
        name,
        prompt,
        builtin: false,
        needs_shelf: false,
    };
    recipes.push(recipe.clone());
    write(paths, &recipes)?;
    Ok(recipe)
}

pub fn update(paths: &Paths, id: &str, name: &str, prompt: &str) -> Result<Recipe> {
    let transaction = crate::paths::metadata_lock(&paths.recipes_path());
    let _write = crate::core::mutex_lock(&transaction);
    let (name, prompt) = validated_recipe_fields(name, prompt)?;
    let mut recipes = list_unlocked(paths);
    let recipe = recipes
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or_else(|| anyhow!("Recipe not found"))?;
    recipe.name = name;
    recipe.prompt = prompt;
    let saved = recipe.clone();
    write(paths, &recipes)?;
    Ok(saved)
}

pub fn delete(paths: &Paths, id: &str) -> Result<()> {
    let transaction = crate::paths::metadata_lock(&paths.recipes_path());
    let _write = crate::core::mutex_lock(&transaction);
    let mut recipes = list_unlocked(paths);
    let before = recipes.len();
    recipes.retain(|r| r.id != id);
    if recipes.len() == before {
        return Err(anyhow!("Recipe not found"));
    }
    write(paths, &recipes)
}

/// Add missing defaults without changing existing Recipes.
pub fn restore_defaults(paths: &Paths) -> Result<Vec<Recipe>> {
    let transaction = crate::paths::metadata_lock(&paths.recipes_path());
    let _write = crate::core::mutex_lock(&transaction);
    let mut recipes = list_unlocked(paths);
    for default in default_recipes() {
        if !recipes.iter().any(|recipe| recipe.id == default.id) {
            recipes.push(default);
        }
    }
    write(paths, &recipes)?;
    Ok(recipes)
}

/// Reset one shipped Recipe, leaving the rest of the collection intact.
pub fn reset_default(paths: &Paths, id: &str) -> Result<Recipe> {
    let transaction = crate::paths::metadata_lock(&paths.recipes_path());
    let _write = crate::core::mutex_lock(&transaction);
    let default = default_recipes()
        .into_iter()
        .find(|recipe| recipe.id == id)
        .ok_or_else(|| anyhow!("Recipe not found"))?;
    let mut recipes = list_unlocked(paths);
    let recipe = recipes
        .iter_mut()
        .find(|recipe| recipe.id == id)
        .ok_or_else(|| anyhow!("Recipe not found"))?;
    *recipe = default.clone();
    write(paths, &recipes)?;
    Ok(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        paths.ensure().unwrap();
        (dir, paths)
    }

    #[test]
    fn first_use_seeds_the_defaults() {
        let (_dir, paths) = paths();
        let recipes = list(&paths);
        assert_eq!(recipes.len(), DEFAULTS.len());
        assert!(recipes.iter().all(|r| r.builtin));
        assert!(recipes.iter().any(|r| r.name == "Reply to a client"));
        assert!(recipes.iter().any(|r| r.id == "document-key-terms"));
        assert!(recipes
            .iter()
            .find(|r| r.id == "one-page-brief")
            .is_some_and(|r| r.needs_shelf));
        // Persisted.
        assert!(paths.recipes_path().exists());
    }

    #[test]
    fn create_delete_and_restore() {
        let (_dir, paths) = paths();
        let mine = create(
            &paths,
            "Weekly update",
            "Write our weekly team update: «notes»",
        )
        .unwrap();
        assert!(!mine.builtin);
        assert_eq!(list(&paths).len(), DEFAULTS.len() + 1);

        // Defaults are deletable too.
        delete(&paths, "translate").unwrap();
        delete(&paths, &mine.id).unwrap();
        let after = list(&paths);
        assert_eq!(after.len(), DEFAULTS.len() - 1);
        assert!(!after.iter().any(|r| r.id == "translate"));

        // Restore adds the missing default.
        let restored = restore_defaults(&paths).unwrap();
        assert_eq!(restored.len(), DEFAULTS.len());
        assert!(restored.iter().any(|r| r.id == "translate"));
        assert!(!restored.iter().any(|r| r.id == mine.id));
        assert!(restored.iter().all(|r| r.builtin));
    }

    #[test]
    fn bulk_restore_preserves_edits_and_custom_recipes() {
        let (_dir, paths) = paths();
        let _ = list(&paths);
        let updated = update(
            &paths,
            "translate",
            "Translate for me",
            "Put this into «language».\n\n«paste the text here»",
        )
        .unwrap();
        assert_eq!(updated.name, "Translate for me");
        assert!(updated.builtin);
        assert_eq!(
            list(&paths)
                .iter()
                .find(|r| r.id == "translate")
                .unwrap()
                .name,
            "Translate for me"
        );

        let mine = create(&paths, "Custom brief", "My prompt").unwrap();
        delete(&paths, "one-page-brief").unwrap();
        let restored = restore_defaults(&paths).unwrap();
        assert_eq!(restored.len(), DEFAULTS.len() + 1);
        assert_eq!(
            restored.iter().find(|r| r.id == "translate").unwrap().name,
            "Translate for me"
        );
        assert_eq!(
            restored.iter().find(|r| r.id == mine.id).unwrap().prompt,
            "My prompt"
        );
        assert!(restored.iter().any(|r| r.id == "one-page-brief"));
        assert_eq!(restore_defaults(&paths).unwrap().len(), restored.len());

        let reset = reset_default(&paths, "translate").unwrap();
        assert_eq!(reset.name, "Translate this");
        assert_eq!(
            list(&paths)
                .iter()
                .find(|r| r.id == mine.id)
                .unwrap()
                .prompt,
            "My prompt"
        );
        assert!(reset_default(&paths, &mine.id).is_err());
        assert_eq!(list(&paths).len(), DEFAULTS.len() + 1);
    }

    #[test]
    fn shipped_copy_follows_locale() {
        let ca = default_recipes_in("ca");
        let es = default_recipes_in("es");
        let en = default_recipes_in("en");
        let translate = |list: &[Recipe]| {
            list.iter()
                .find(|r| r.id == "translate")
                .unwrap()
                .name
                .clone()
        };
        assert_eq!(translate(&en), "Translate this");
        assert_ne!(translate(&ca), "Translate this");
        assert_ne!(translate(&es), "Translate this");
        assert!(
            ca.iter()
                .find(|r| r.id == "one-page-brief")
                .unwrap()
                .needs_shelf
        );
    }

    #[test]
    fn create_validates_input() {
        let (_dir, paths) = paths();
        assert!(create(&paths, "", "prompt").is_err());
        assert!(create(&paths, "name", "  ").is_err());
        let too_long = "a".repeat(crate::limits::PROMPT_MAX_CHARS + 1);
        assert!(create(&paths, "Long", &too_long).is_err());
        let fits = "a".repeat(crate::limits::PROMPT_MAX_CHARS);
        assert!(create(&paths, "Fits", &fits).is_ok());
    }

    #[test]
    fn old_contract_key_terms_id_is_renamed() {
        let (_dir, paths) = paths();
        let _ = list(&paths);
        let mut recipes = list(&paths);
        if let Some(recipe) = recipes.iter_mut().find(|r| r.id == "document-key-terms") {
            recipe.id = "contract-key-terms".into();
        }
        write(&paths, &recipes).unwrap();
        let loaded = list(&paths);
        assert!(loaded.iter().any(|r| r.id == "document-key-terms"));
        assert!(!loaded.iter().any(|r| r.id == "contract-key-terms"));
    }
}
