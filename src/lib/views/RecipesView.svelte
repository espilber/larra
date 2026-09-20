<script lang="ts">
  import { api, type Recipe } from "$lib/api";
  import { previewParts } from "$lib/placeholders";
  import { notifyInvokeError, startRecipe } from "$lib/stores.svelte";
  import { PROMPT_MAX_CHARS } from "$lib/text-cap";
  import {
    BookmarkPlus,
    ChefHat,
    Trash2,
    ArrowUpRight,
    RotateCcw,
    X,
    Pencil,
  } from "@lucide/svelte";
  import { confirmDanger } from "$lib/native-dialog";
  import { t } from "$lib/i18n.svelte";
  import { shot } from "$lib/shot-control.svelte";

  let recipes = $state<Recipe[]>([]);
  let formOpen = $state(shot.recipeNew || import.meta.env.VITE_START_RECIPE === "new");
  let editingId = $state<string | null>(null);
  let formName = $state("");
  let formPrompt = $state("");

  async function refresh() {
    recipes = await api.recipesList();
  }

  $effect(() => {
    refresh().catch(notifyInvokeError);
  });

  function use(recipe: Recipe) {
    startRecipe(recipe.prompt);
  }

  function beginCreate() {
    editingId = null;
    formName = "";
    formPrompt = "";
    formOpen = true;
  }

  function beginEdit(recipe: Recipe, event: MouseEvent) {
    event.stopPropagation();
    editingId = recipe.id;
    formName = recipe.name;
    formPrompt = recipe.prompt;
    formOpen = true;
  }

  function cancelForm() {
    formOpen = false;
    editingId = null;
    formName = "";
    formPrompt = "";
  }

  async function remove(recipe: Recipe, event: MouseEvent) {
    event.stopPropagation();
    const ok = await confirmDanger(
      t("recipes.deleteConfirm", { name: recipe.name }),
      t("chat.delete"),
    );
    if (!ok) return;
    try {
      await api.recipeDelete(recipe.id);
      if (editingId === recipe.id) cancelForm();
      await refresh();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function save() {
    try {
      if (editingId) {
        await api.recipeUpdate(editingId, formName, formPrompt);
      } else {
        await api.recipeCreate(formName, formPrompt);
      }
      cancelForm();
      await refresh();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function restoreDefaults() {
    try {
      recipes = await api.recipesRestoreDefaults();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function resetDefault() {
    const id = editingId;
    if (!id || !(await confirmDanger(t("recipes.resetConfirm"), t("recipes.resetAction")))) return;
    try {
      const recipe = await api.recipeResetDefault(id);
      await refresh();
      if (editingId === id) {
        formName = recipe.name;
        formPrompt = recipe.prompt;
      }
    } catch (error) {
      notifyInvokeError(error);
    }
  }
</script>

<div class="mx-auto max-w-[860px] px-4 py-5 min-[900px]:px-8 min-[900px]:py-8">
  <div class="mb-6 flex flex-wrap items-end justify-between gap-4">
    <div class="min-w-0 flex-1 basis-64">
      <h1 class="text-[22px] font-semibold text-ink">{t("recipes.title")}</h1>
      <p class="mt-0.5 text-[13px] text-ink-soft">
        {t("recipes.lede")}
      </p>
    </div>
    <button type="button" class="btn-primary" onclick={beginCreate}>
      <BookmarkPlus size={15} aria-hidden="true" />
      {t("recipes.newRecipe")}
    </button>
  </div>

  {#if formOpen}
    <div class="card mb-5 px-5 py-4">
      <div class="mb-3 flex items-center justify-between">
        <span class="text-[13.5px] font-semibold text-ink"
          >{editingId ? t("recipes.editRecipe") : t("recipes.newRecipe")}</span
        >
        <button
          type="button"
          class="btn-ghost !p-1.5"
          aria-label={editingId ? t("recipes.cancelEdit") : t("recipes.cancelNew")}
          onclick={cancelForm}><X size={14} aria-hidden="true" /></button
        >
      </div>
      <label class="sr-only" for="recipe-name">{t("recipes.name")}</label>
      <!-- svelte-ignore a11y_autofocus -->
      <input
        id="recipe-name"
        name="recipe-name"
        class="input mb-2.5"
        placeholder={t("recipes.namePlaceholder")}
        bind:value={formName}
        autofocus={!import.meta.env.VITE_SNAPSHOT_PATH}
      />
      <label class="sr-only" for="recipe-prompt">{t("recipes.prompt")}</label>
      <textarea
        id="recipe-prompt"
        name="recipe-prompt"
        class="input min-h-28 cursor-text resize-y select-text"
        placeholder={t("recipes.promptPlaceholder")}
        maxlength={PROMPT_MAX_CHARS}
        bind:value={formPrompt}></textarea>
      <div class="mt-2.5 flex flex-wrap justify-end gap-2">
        {#if recipes.some((recipe) => recipe.id === editingId && recipe.builtin)}
          <button type="button" class="btn-ghost mr-auto" onclick={resetDefault}>
            <RotateCcw size={13} aria-hidden="true" />{t("recipes.resetAction")}
          </button>
        {/if}
        <button type="button" class="btn-ghost" onclick={cancelForm}>{t("recipes.cancel")}</button>
        <button
          type="button"
          class="btn-amber"
          onclick={save}
          disabled={!formName.trim() || !formPrompt.trim()}
        >
          {t("recipes.save")}
        </button>
      </div>
    </div>
  {/if}

  <div class="grid grid-cols-1 gap-4 min-[900px]:grid-cols-2">
    {#each recipes as recipe (recipe.id)}
      <div
        class="card group relative flex flex-col px-5 py-4 text-left hover:shadow-pop dark:hover:shadow-none"
      >
        <div class="mb-1.5 flex items-center gap-2.5">
          <button
            type="button"
            class="flex min-w-0 flex-1 items-center gap-2.5 text-left"
            onclick={() => use(recipe)}
          >
            <span class="rounded-lg bg-navy-900 p-2 text-mint"
              ><ChefHat size={14} aria-hidden="true" /></span
            >
            <span class="flex-1 truncate text-[14px] font-semibold text-ink">{recipe.name}</span>
            <span
              class="rounded-md p-1.5 text-navy-600 dark:text-navy-300"
              title={t("recipes.use")}
            >
              <ArrowUpRight size={14} aria-hidden="true" />
            </span>
          </button>
          <button
            type="button"
            class="rounded-md p-1.5 text-ink-faint hover:bg-navy-50 hover:text-ink dark:hover:bg-white/6"
            aria-label={t("recipes.edit")}
            title={t("recipes.edit")}
            onclick={(e) => beginEdit(recipe, e)}
          >
            <Pencil size={13} aria-hidden="true" />
          </button>
          <button
            type="button"
            class="rounded-md p-1.5 text-ink-faint hover:bg-red-50 hover:text-red-700 dark:hover:bg-red-400/10 dark:hover:text-red-400"
            aria-label={t("recipes.delete")}
            title={t("recipes.delete")}
            onclick={(e) => remove(recipe, e)}
          >
            <Trash2 size={13} aria-hidden="true" />
          </button>
        </div>
        <button
          type="button"
          class="line-clamp-3 text-left text-[12px] leading-relaxed text-ink-soft"
          aria-label={t("recipes.useNamed", { name: recipe.name })}
          onclick={() => use(recipe)}
        >
          {#each previewParts(recipe.prompt) as part}
            {#if part.ph}<span
                class="rounded-md bg-navy-100 px-1 py-px font-medium text-navy-800 dark:bg-navy-500/15 dark:text-navy-200 dark:inset-ring dark:inset-ring-navy-500/45"
                >{part.text}</span
              >{:else}{part.text}{/if}
          {/each}
        </button>
      </div>
    {:else}
      <div class="card flex flex-col items-center px-8 py-12 text-center min-[900px]:col-span-2">
        <ChefHat size={22} class="mb-2 text-ink-faint" aria-hidden="true" />
        <p class="text-[13.5px] font-medium text-ink">{t("recipes.emptyTitle")}</p>
        <p class="mt-1 text-[12.5px] text-ink-soft">
          {t("recipes.emptyBody")}
        </p>
      </div>
    {/each}
  </div>

  <div class="mt-5 flex justify-center">
    <button type="button" class="btn-ghost !px-3 !py-1.5 !text-[12px]" onclick={restoreDefaults}>
      <RotateCcw size={12.5} aria-hidden="true" />
      {t("recipes.restore")}
    </button>
  </div>
</div>
