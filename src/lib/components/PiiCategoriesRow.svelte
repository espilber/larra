<script lang="ts">
  import { PII_CATEGORY_ORDER, piiLabel } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { focusTrap } from "$lib/focus-trap";
  import { countFittingChips, visibleSyncedFolders } from "$lib/synced-folders";
  import { ShieldCheck } from "@lucide/svelte";

  let {
    filesWithPii,
    categories,
    filterPii,
    onFilter,
  }: {
    filesWithPii: number;
    categories: Record<string, number>;
    filterPii: string | null;
    onFilter: (value: string | null) => void;
  } = $props();

  const PII_ORDER = PII_CATEGORY_ORDER;

  const items = $derived(
    PII_ORDER.filter((category) => categories[category]).map((category) => ({
      category,
      count: categories[category] ?? 0,
    })),
  );

  let rowEl = $state<HTMLDivElement | null>(null);
  let measureEl = $state<HTMLDivElement | null>(null);
  let visibleCount = $state(Number.MAX_SAFE_INTEGER);
  let moreOpen = $state(false);

  const visibleItems = $derived(
    visibleSyncedFolders(items, visibleCount, (item) => filterPii === item.category),
  );
  const hiddenCount = $derived(Math.max(0, items.length - visibleItems.length));

  function measure() {
    const row = rowEl;
    const measureRoot = measureEl;
    if (!row || !measureRoot) return;
    const chips = [...measureRoot.querySelectorAll<HTMLElement>("[data-pii-chip]")];
    const more = measureRoot.querySelector<HTMLElement>("[data-pii-more]");
    const summary = measureRoot.querySelector<HTMLElement>("[data-pii-summary]");
    if (row.clientWidth === 0) return;
    const gap = 6;
    const available = row.clientWidth - (summary?.offsetWidth ?? 0) - gap;
    if (chips.length === 0) {
      visibleCount = 0;
      return;
    }
    const next = countFittingChips(
      chips.map((chip) => chip.offsetWidth),
      available,
      more?.offsetWidth ?? 28,
      gap,
    );
    visibleCount = next;
    if (next >= items.length) moreOpen = false;
  }

  $effect(() => {
    void items;
    void filterPii;
    void filesWithPii;
    void measureEl;
    const row = rowEl;
    if (!row) return;
    const frame = requestAnimationFrame(measure);
    const observer = new ResizeObserver(() => measure());
    observer.observe(row);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
    };
  });

  function toggle(value: string) {
    onFilter(filterPii === value ? null : value);
    moreOpen = false;
  }
</script>

{#snippet categoryChip(category: string, count: number)}
  <button
    type="button"
    data-pii-chip
    class="chip {filterPii === category
      ? 'bg-navy-900 text-white'
      : 'bg-paper-soft text-ink-soft hover:bg-navy-100/70 hover:text-navy-800 dark:hover:bg-white/8 dark:hover:text-ink'}"
    aria-pressed={filterPii === category}
    onclick={() => toggle(category)}
  >
    {count}
    {piiLabel(category, count)}
  </button>
{/snippet}

{#snippet summaryChip()}
  <button
    type="button"
    data-pii-summary
    class="chip shrink-0 py-1 pr-2.5 pl-1.5 {filterPii === 'any'
      ? 'bg-navy-900 text-white'
      : 'bg-navy-100/70 text-navy-800 hover:bg-navy-200/70 dark:bg-white/10 dark:text-navy-100 dark:hover:bg-white/15'}"
    aria-pressed={filterPii === "any"}
    onclick={() => toggle("any")}
  >
    <ShieldCheck size={12} class="shrink-0" aria-hidden="true" />
    {t("pii.filesContain", { count: filesWithPii })}
  </button>
{/snippet}

<div class="relative min-w-0 flex-1">
  <div bind:this={rowEl} class="flex min-w-0 flex-nowrap items-center gap-1.5">
    {@render summaryChip()}
    {#each visibleItems as item (item.category)}
      {@render categoryChip(item.category, item.count)}
    {/each}
    {#if hiddenCount > 0}
      <div class="relative shrink-0">
        <button
          type="button"
          class="chip shrink-0 border border-paper-line bg-surface text-ink-soft hover:bg-navy-100/70 dark:hover:bg-white/8"
          aria-label={t("pii.more")}
          aria-haspopup="dialog"
          aria-expanded={moreOpen}
          aria-controls="pii-categories-menu"
          onclick={() => (moreOpen = !moreOpen)}
        >
          ...
        </button>
        {#if moreOpen}
          <div
            class="fixed inset-0 z-20"
            role="presentation"
            onclick={() => (moreOpen = false)}
          ></div>
          <div
            id="pii-categories-menu"
            class="absolute right-0 bottom-full z-30 mb-1.5 max-h-64 min-w-[220px] overflow-y-auto rounded-xl border border-paper-line bg-surface p-2 shadow-pop dark:shadow-none"
            role="dialog"
            aria-label={t("pii.heading")}
            tabindex="-1"
            use:focusTrap
            onkeydown={(event) => event.key === "Escape" && (moreOpen = false)}
          >
            <p class="label px-1 pb-1.5 !text-[10px]">{t("pii.heading")}</p>
            <ul class="flex flex-col gap-1.5">
              {#each items as item (item.category)}
                <li class="min-w-0">{@render categoryChip(item.category, item.count)}</li>
              {/each}
            </ul>
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <div
    bind:this={measureEl}
    class="pointer-events-none invisible absolute top-0 left-0 flex w-max flex-nowrap items-center gap-1.5"
    inert
    aria-hidden="true"
  >
    {@render summaryChip()}
    {#each items as item (item.category)}
      {@render categoryChip(item.category, item.count)}
    {/each}
    <span data-pii-more class="chip shrink-0 border border-paper-line">...</span>
  </div>
</div>
