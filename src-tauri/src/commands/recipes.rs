//! Saved-prompt library.

use std::sync::Arc;
use tauri::State;

use super::{friendly, require_id, CmdResult};
use crate::core::Ctx;

/// List saved recipes.
#[tauri::command]
pub fn recipes_list(ctx: State<'_, Arc<Ctx>>) -> Vec<crate::recipes::Recipe> {
    crate::recipes::list(&ctx.paths)
}

/// Create a recipe from a name and prompt.
#[tauri::command]
pub fn recipe_create(
    ctx: State<'_, Arc<Ctx>>,
    name: String,
    prompt: String,
) -> CmdResult<crate::recipes::Recipe> {
    crate::recipes::create(&ctx.paths, &name, &prompt).map_err(friendly)
}

/// Update a recipe's name and prompt.
#[tauri::command]
pub fn recipe_update(
    ctx: State<'_, Arc<Ctx>>,
    id: String,
    name: String,
    prompt: String,
) -> CmdResult<crate::recipes::Recipe> {
    require_id(&id)?;
    crate::recipes::update(&ctx.paths, &id, &name, &prompt).map_err(friendly)
}

/// Delete a recipe by id.
#[tauri::command]
pub fn recipe_delete(ctx: State<'_, Arc<Ctx>>, id: String) -> CmdResult<()> {
    require_id(&id)?;
    crate::recipes::delete(&ctx.paths, &id).map_err(friendly)
}

/// Add missing shipped Recipes without replacing edits or custom Recipes.
#[tauri::command]
pub fn recipes_restore_defaults(
    ctx: State<'_, Arc<Ctx>>,
) -> CmdResult<Vec<crate::recipes::Recipe>> {
    crate::recipes::restore_defaults(&ctx.paths).map_err(friendly)
}

#[tauri::command]
pub fn recipe_reset_default(
    ctx: State<'_, Arc<Ctx>>,
    id: String,
) -> CmdResult<crate::recipes::Recipe> {
    require_id(&id)?;
    crate::recipes::reset_default(&ctx.paths, &id).map_err(friendly)
}
