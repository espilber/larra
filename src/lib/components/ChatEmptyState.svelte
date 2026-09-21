<script lang="ts">
  import { api, type Recipe } from "$lib/api";
  import { recipeNeedsShelf } from "$lib/placeholders";
  import { t } from "$lib/i18n.svelte";
  import { app, chatState, fillDraft } from "$lib/stores.svelte";
  import ConversationFace from "./ConversationFace.svelte";
  import mark from "../../assets/larra-mark.png";

  let { avatarId = null }: { avatarId?: string | null } = $props();

  let recipes = $state<Recipe[]>([]);
  const hasModel = $derived(!!app.settings?.activeModel);

  $effect(() => {
    if (!hasModel) {
      recipes = [];
      return;
    }
    api
      .recipesList()
      .then((list) => (recipes = list))
      .catch(() => {
        recipes = [];
      });
  });

  const shown = $derived.by(() => {
    const hasShelf = !!chatState.selectedShelfId;
    const withShelf = recipes.filter((recipe) => recipeNeedsShelf(recipe));
    const without = recipes.filter((recipe) => !recipeNeedsShelf(recipe));
    const ordered = hasShelf ? [...withShelf, ...without] : without;
    return ordered.slice(0, 6);
  });
</script>

<div class="flex h-full flex-col items-center justify-center px-4 py-4 min-[900px]:px-8">
  {#if avatarId}
    <div class="mb-3">
      <ConversationFace {avatarId} size="hero" />
    </div>
  {:else}
    <img src={mark} alt="Larra" class="mb-3 w-[100px] rounded-2xl" />
  {/if}
  {#if !hasModel}
    <h2 class="text-[19px] font-semibold text-ink">{t("chat.installFirstTitle")}</h2>
    <p class="mt-1 mb-6 max-w-md text-center text-[13px] text-ink-soft">
      {t("chat.installFirstBody")}
    </p>
    <button
      type="button"
      class="btn-amber focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-800"
      onclick={() => (app.view = "settings")}
    >
      {t("chat.install")}
    </button>
  {:else}
    <h2 class="text-[19px] font-semibold text-ink">{t("chat.startTitle")}</h2>
    <p class="mt-1 mb-6 max-w-md text-center text-[13px] text-ink-soft">
      {t("chat.startBody")}
    </p>
  {/if}
  {#if hasModel && shown.length > 0}
    <div class="grid w-full max-w-xl grid-cols-1 gap-2 min-[900px]:grid-cols-2">
      {#each shown as recipe (recipe.id)}
        <button
          type="button"
          class="card px-3.5 py-3 text-left text-[12.5px] text-ink-soft hover:border-navy-300 hover:text-ink"
          onclick={() => fillDraft(recipe.prompt)}
        >
          {recipe.name}
        </button>
      {/each}
    </div>
  {:else if hasModel && recipes.length > 0 && !chatState.selectedShelfId && app.shelves.length > 0}
    <p class="max-w-md text-center text-[12.5px] text-ink-faint">
      {t("chat.chooseShelfForRecipes")}
    </p>
  {/if}
</div>
