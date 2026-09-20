<script lang="ts">
  import { api, formatBytes, formatCount, formatReleased, type ModelSearchResult } from "$lib/api";
  import {
    EXPLORE_PAGE_SIZE,
    EXPLORE_SORTS,
    chipSortActive,
    columnAriaSort,
    defaultExploreSortDir,
    exploreSortLabel,
    nextExploreSort,
    normalizeExploreQuery,
    parseExploreRepoQuery,
    sortExploreResults,
    visibleExploreCount,
    type ExploreColumn,
    type ExploreSort,
    type ExploreSortDir,
  } from "$lib/explore-models";
  import { focusTrap } from "$lib/focus-trap";
  import { dialogPanel, overlay } from "$lib/motion";
  import { notifyInvokeError } from "$lib/stores.svelte";
  import { t } from "$lib/i18n.svelte";
  import { ArrowDown, ArrowUp, ArrowUpDown, Download, Info, Search, X } from "@lucide/svelte";
  import { onMount } from "svelte";
  import ModelInfoModal from "$lib/components/ModelInfoModal.svelte";

  let {
    installing = false,
    onClose,
    onInstall,
  }: {
    installing?: boolean;
    onClose: () => void;
    onInstall: (result: ModelSearchResult) => void;
  } = $props();

  let query = $state("");
  let searching = $state(true);
  let results = $state<ModelSearchResult[] | null>(null);
  let failed = $state(false);
  let sort = $state<ExploreSort>("best");
  let sortDir = $state<ExploreSortDir>("desc");
  let page = $state(1);
  let info = $state<ModelSearchResult | null>(null);
  let searchGen = 0;

  const sorted = $derived(results ? sortExploreResults(results, sort, sortDir) : []);
  const visibleCount = $derived(visibleExploreCount(sorted.length, page));
  const visible = $derived(sorted.slice(0, visibleCount));
  const remaining = $derived(Math.max(0, sorted.length - visibleCount));
  const showSkeleton = $derived(searching && results === null);
  const statusText = $derived.by(() => {
    if (showSkeleton) return t("explore.lookingUp");
    if (failed) return t("explore.couldntReach");
    if (results !== null && results.length === 0) {
      return query.trim() ? t("explore.nothingFound") : t("explore.noneToShow");
    }
    if (results !== null) {
      return remaining > 0
        ? t("explore.showingOf", { visible: visibleCount, total: sorted.length })
        : sorted.length === 1
          ? t("explore.oneAi")
          : t("explore.manyAis", { count: sorted.length });
    }
    return "";
  });

  onMount(() => {
    void runSearch("");
  });

  async function runSearch(nextQuery: string) {
    const gen = ++searchGen;
    searching = true;
    failed = false;
    page = 1;
    results = null;
    try {
      const found = await api.modelsSearch(nextQuery);
      if (gen !== searchGen) return;
      results = found;
    } catch (error) {
      if (gen !== searchGen) return;
      notifyInvokeError(error);
      results = [];
      failed = true;
    } finally {
      if (gen === searchGen) searching = false;
    }
  }

  function submitSearch() {
    const next = normalizeExploreQuery(query);
    query = next;
    void runSearch(next);
  }

  function onSearchPaste(event: ClipboardEvent) {
    const repo = parseExploreRepoQuery(event.clipboardData?.getData("text") ?? "");
    if (!repo) return;
    event.preventDefault();
    query = repo;
    void runSearch(repo);
  }

  function applyChip(next: ExploreSort) {
    sort = next;
    sortDir = defaultExploreSortDir(next);
    page = 1;
  }

  function clickColumn(column: ExploreColumn) {
    const next = nextExploreSort(sort, sortDir, column);
    sort = next.sort;
    sortDir = next.dir;
    page = 1;
  }

  function releasedLabel(released?: string): string {
    if (!released) return "—";
    return formatReleased(released.length >= 7 ? released.slice(0, 7) : released);
  }

  function fitKind(fits?: boolean): "ok" | "no" | "na" {
    switch (fits) {
      case true:
        return "ok";
      case false:
        return "no";
      case undefined:
        return "na";
      default: {
        const _never: never = fits;
        return _never;
      }
    }
  }

  function onDialogKey(event: KeyboardEvent) {
    if (event.key !== "Escape") return;
    if (info) {
      event.stopPropagation();
      info = null;
      return;
    }
    onClose();
  }
</script>

<div
  class="fixed inset-0 z-40 flex items-center justify-center bg-navy-950/25 p-3 sm:p-6 dark:bg-black/50"
  role="dialog"
  aria-modal="true"
  aria-labelledby="explore-models-title"
  aria-describedby="explore-models-lede"
  aria-busy={searching}
  id="explore-models-dialog"
  tabindex="-1"
  use:focusTrap
  transition:overlay
  onclick={(e) => e.target === e.currentTarget && !info && onClose()}
  onkeydown={onDialogKey}
>
  <div
    class="card flex max-h-[min(48rem,92vh)] w-full max-w-5xl flex-col overflow-hidden shadow-pop dark:shadow-none"
    in:dialogPanel
  >
    <div class="relative px-5 pt-5 pb-4">
      <div class="pr-10">
        <h2 id="explore-models-title" class="text-[16px] font-semibold text-ink">
          {t("explore.title")}
        </h2>
        <p
          id="explore-models-lede"
          class="mt-1 max-w-[62ch] text-[12.5px] leading-snug text-ink-soft"
        >
          {t("explore.lede")}
        </p>
      </div>
      <div class="mt-4 flex flex-col gap-3">
        <div class="flex gap-2">
          <div class="relative min-w-0 flex-1">
            <Search
              size={13.5}
              class="absolute top-1/2 left-3 -translate-y-1/2 text-ink-faint"
              aria-hidden="true"
            />
            <label class="sr-only" for="explore-model-search">{t("explore.search")}</label>
            <input
              id="explore-model-search"
              name="explore-model-search"
              class="input !pl-9"
              placeholder={t("explore.searchPlaceholder")}
              autocomplete="off"
              spellcheck="false"
              bind:value={query}
              onkeydown={(e) => e.key === "Enter" && submitSearch()}
              onpaste={onSearchPaste}
            />
          </div>
          <button type="button" class="btn-primary shrink-0" onclick={submitSearch}>
            {searching ? t("explore.searching") : t("explore.searchAction")}
          </button>
        </div>

        <div class="flex flex-wrap gap-1.5" role="group" aria-label={t("explore.sort")}>
          {#each EXPLORE_SORTS as option (option.id)}
            <button
              type="button"
              class="inline-flex shrink-0 cursor-default items-center rounded-full px-2.5 py-1.5 text-[12px] leading-none font-medium whitespace-nowrap
                ring-1 ring-navy-950/10 aria-pressed:bg-navy-900 aria-pressed:text-white aria-pressed:ring-navy-900
                dark:ring-white/10 dark:aria-pressed:bg-white dark:aria-pressed:text-navy-950 dark:aria-pressed:ring-white"
              aria-pressed={chipSortActive(option.id, sort, sortDir)}
              onclick={() => applyChip(option.id)}
            >
              {exploreSortLabel(option.id)}
            </button>
          {/each}
        </div>
      </div>
      <button
        type="button"
        class="btn-ghost absolute top-4 right-4 !p-1.5"
        aria-label={t("explore.close")}
        onclick={onClose}
      >
        <X size={15} aria-hidden="true" />
      </button>
    </div>

    <div
      class="min-h-0 flex-1 overflow-x-hidden overflow-y-auto border-t border-navy-950/10 dark:border-white/10"
    >
      {#if showSkeleton}
        <div class="px-5 py-4" aria-live="polite">
          <p class="text-[13px] font-medium text-ink">{statusText}</p>
          <p class="mt-1 text-[12px] text-ink-soft">{t("explore.onlySearchWords")}</p>
          <table class="mt-4 w-full" aria-hidden="true">
            <tbody class="divide-y divide-navy-950/10 dark:divide-white/10">
              {#each [1, 2, 3, 4, 5, 6, 7] as row (row)}
                <tr>
                  <td class="py-2.5 pr-3">
                    <div
                      class="h-3.5 w-[min(100%,14rem)] rounded-md bg-paper-soft motion-safe:animate-pulse dark:bg-white/10"
                    ></div>
                  </td>
                  <td class="hidden py-2.5 pr-3 sm:table-cell">
                    <div
                      class="h-3 w-20 rounded-md bg-paper-soft motion-safe:animate-pulse dark:bg-white/10"
                    ></div>
                  </td>
                  <td class="hidden py-2.5 pr-3 md:table-cell">
                    <div
                      class="h-3 w-14 rounded-md bg-paper-soft motion-safe:animate-pulse dark:bg-white/10"
                    ></div>
                  </td>
                  <td class="hidden py-2.5 pr-3 lg:table-cell">
                    <div
                      class="h-3 w-12 rounded-md bg-paper-soft motion-safe:animate-pulse dark:bg-white/10"
                    ></div>
                  </td>
                  <td class="py-2.5">
                    <div
                      class="ml-auto h-3 w-16 rounded-md bg-paper-soft motion-safe:animate-pulse dark:bg-white/10"
                    ></div>
                  </td>
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {:else if visible.length > 0}
        <table class="w-full table-fixed text-left text-[13px]">
          <caption class="sr-only">{t("explore.caption")}</caption>
          <colgroup>
            <col />
            <col class="w-[7.5rem]" />
            <col class="w-[5.5rem]" />
            <col class="w-[6.75rem]" />
            <col class="w-[6.5rem]" />
            <col class="w-[13rem]" />
          </colgroup>
          <thead
            class="sticky top-0 z-10 border-b border-navy-950/15 bg-paper-soft text-[12px] font-semibold text-ink dark:border-white/15 dark:bg-white/10"
          >
            <tr>
              <th scope="col" class="px-5 py-3 font-semibold whitespace-nowrap"
                >{t("explore.colName")}</th
              >
              <th scope="col" class="px-3 py-3 font-semibold whitespace-nowrap"
                >{t("explore.colPublisher")}</th
              >
              <th
                scope="col"
                class="px-3 py-3 font-semibold whitespace-nowrap"
                aria-sort={columnAriaSort("size", sort, sortDir)}
              >
                <button
                  type="button"
                  class="inline-flex items-center gap-1 text-ink-soft aria-[current=true]:text-ink"
                  aria-current={sort === "size"}
                  onclick={() => clickColumn("size")}
                >
                  {t("explore.colSize")}
                  {#if sort === "size" && sortDir === "asc"}
                    <ArrowUp size={11} aria-hidden="true" />
                  {:else if sort === "size"}
                    <ArrowDown size={11} aria-hidden="true" />
                  {:else}
                    <ArrowUpDown size={11} class="opacity-45" aria-hidden="true" />
                  {/if}
                </button>
              </th>
              <th
                scope="col"
                class="px-3 py-3 font-semibold whitespace-nowrap"
                aria-sort={columnAriaSort("downloads", sort, sortDir)}
              >
                <button
                  type="button"
                  class="inline-flex items-center gap-1 text-ink-soft aria-[current=true]:text-ink"
                  aria-current={sort === "downloads"}
                  onclick={() => clickColumn("downloads")}
                >
                  {t("explore.colDownloads")}
                  {#if sort === "downloads" && sortDir === "asc"}
                    <ArrowUp size={11} aria-hidden="true" />
                  {:else if sort === "downloads"}
                    <ArrowDown size={11} aria-hidden="true" />
                  {:else}
                    <ArrowUpDown size={11} class="opacity-45" aria-hidden="true" />
                  {/if}
                </button>
              </th>
              <th
                scope="col"
                class="px-3 py-3 font-semibold whitespace-nowrap"
                aria-sort={columnAriaSort("released", sort, sortDir)}
              >
                <button
                  type="button"
                  class="inline-flex items-center gap-1 text-ink-soft aria-[current=true]:text-ink"
                  aria-current={sort === "released"}
                  onclick={() => clickColumn("released")}
                >
                  {t("explore.colReleased")}
                  {#if sort === "released" && sortDir === "asc"}
                    <ArrowUp size={11} aria-hidden="true" />
                  {:else if sort === "released"}
                    <ArrowDown size={11} aria-hidden="true" />
                  {:else}
                    <ArrowUpDown size={11} class="opacity-45" aria-hidden="true" />
                  {/if}
                </button>
              </th>
              <th scope="col" class="px-5 py-3 text-right font-semibold whitespace-nowrap">
                <span class="sr-only">{t("explore.actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody class="divide-y divide-navy-950/10 dark:divide-white/10">
            {#each visible as result (result.source + result.reference)}
              {@const fit = fitKind(result.fits)}
              <tr class="bg-surface">
                <th scope="row" class="max-w-0 px-5 py-2.5 font-medium text-ink">
                  <div class="flex min-w-0 items-center gap-1.5">
                    <span class="min-w-0 truncate" title={result.name}>{result.name}</span>
                    {#if result.official}
                      <span
                        class="shrink-0 rounded-full bg-navy-100 px-2 py-0.5 text-[10px] font-semibold text-navy-800 dark:bg-white/10 dark:text-navy-100"
                      >
                        {t("explore.official")}
                      </span>
                    {/if}
                    {#if fit === "ok"}
                      <span
                        class="shrink-0 rounded-full bg-ready px-2 py-0.5 text-[10px] font-semibold text-ready-ink dark:bg-navy-200/20 dark:text-navy-200"
                      >
                        {t("explore.fits")}
                      </span>
                    {:else if fit === "no"}
                      <span
                        class="shrink-0 rounded-full bg-paper-soft px-2 py-0.5 text-[10px] font-semibold text-ink-faint dark:bg-white/8"
                      >
                        {t("explore.tooLarge")}
                      </span>
                    {/if}
                  </div>
                </th>
                <td
                  class="truncate px-3 py-2.5 text-ink-soft"
                  title={result.publisher ?? undefined}
                >
                  {result.publisher ?? "—"}
                </td>
                <td class="px-3 py-2.5 whitespace-nowrap text-ink-soft tabular-nums">
                  {result.sizeBytes != null ? formatBytes(result.sizeBytes) : "—"}
                </td>
                <td class="px-3 py-2.5 whitespace-nowrap text-ink-soft tabular-nums">
                  {result.downloads != null && result.downloads > 0
                    ? formatCount(result.downloads)
                    : "—"}
                </td>
                <td class="px-3 py-2.5 whitespace-nowrap text-ink-soft">
                  {releasedLabel(result.released)}
                </td>
                <td class="px-5 py-2.5">
                  <div class="flex flex-nowrap justify-end gap-1.5">
                    <button
                      type="button"
                      class="btn-ghost !px-2 !py-1.5 !text-[12px] whitespace-nowrap"
                      aria-haspopup="dialog"
                      aria-expanded={info?.reference === result.reference &&
                        info?.source === result.source}
                      aria-controls={info?.reference === result.reference
                        ? "model-info-dialog"
                        : undefined}
                      onclick={() => (info = result)}
                    >
                      <Info size={12.5} aria-hidden="true" />
                      {t("explore.moreInfo")}
                    </button>
                    <button
                      type="button"
                      class="btn-outline !py-1.5 !pr-2.5 !pl-1.5 !text-[12px] whitespace-nowrap"
                      onclick={() => onInstall(result)}
                      disabled={installing || fit === "no"}
                    >
                      <Download size={12.5} aria-hidden="true" />
                      {t("explore.install")}
                    </button>
                  </div>
                </td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else}
        <p class="px-5 py-10 text-center text-[13px] text-ink-soft" aria-live="polite">
          {statusText}
        </p>
      {/if}
    </div>

    <div
      class="flex items-center justify-between gap-3 border-t border-navy-950/10 px-5 py-3 dark:border-white/10"
    >
      <p class="text-[12px] text-ink-faint" aria-live="polite">
        {#if visible.length > 0}{statusText}{/if}
      </p>
      {#if remaining > 0}
        <button type="button" class="btn-outline !py-1.5 !text-[12px]" onclick={() => (page += 1)}>
          {t("explore.seeMore")}
          <span class="text-ink-faint"
            >{t("explore.moreCount", {
              count: Math.min(remaining, EXPLORE_PAGE_SIZE),
            })}</span
          >
        </button>
      {/if}
    </div>
  </div>
</div>

{#if info}
  <ModelInfoModal
    result={info}
    {installing}
    onClose={() => (info = null)}
    onInstall={() => {
      const selected = info;
      if (!selected) return;
      onInstall(selected);
      info = null;
    }}
  />
{/if}
