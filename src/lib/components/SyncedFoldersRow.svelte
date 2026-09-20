<script lang="ts">
  import type { LinkedView } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { linkedFolderName, linkedFolderSourceId } from "$lib/files";
  import { focusTrap } from "$lib/focus-trap";
  import { countFittingChips, visibleSyncedFolders } from "$lib/synced-folders";
  import { FolderSymlink, X } from "@lucide/svelte";

  let {
    folders,
    filterSource,
    onFilter,
    onUnlink,
  }: {
    folders: LinkedView[];
    filterSource: string | null;
    onFilter: (name: string | null) => void;
    onUnlink: (linked: LinkedView) => void;
  } = $props();

  let rowEl = $state<HTMLDivElement | null>(null);
  let measureEl = $state<HTMLDivElement | null>(null);
  let visibleCount = $state(Number.MAX_SAFE_INTEGER);
  let moreOpen = $state(false);

  const folderName = (linked: LinkedView) => linkedFolderName(linked);
  const isSelected = (linked: LinkedView) => {
    const name = folderName(linked);
    return filterSource === name || filterSource === linked.label;
  };

  const visibleFolders = $derived(visibleSyncedFolders(folders, visibleCount, isSelected));
  const hiddenCount = $derived(Math.max(0, folders.length - visibleFolders.length));

  function measure() {
    const row = rowEl;
    const measureRoot = measureEl;
    if (!row || !measureRoot) return;
    const chips = [...measureRoot.querySelectorAll<HTMLElement>("[data-synced-chip]")];
    const more = measureRoot.querySelector<HTMLElement>("[data-synced-more]");
    const label = measureRoot.querySelector<HTMLElement>("[data-synced-label]");
    if (row.clientWidth === 0 || chips.length === 0) return;
    const gap = 6;
    const available = row.clientWidth - (label?.offsetWidth ?? 0) - gap;
    const next = countFittingChips(
      chips.map((chip) => chip.offsetWidth),
      available,
      more?.offsetWidth ?? 28,
      gap,
    );
    visibleCount = next;
    if (next >= folders.length) moreOpen = false;
  }

  $effect(() => {
    void folders;
    void filterSource;
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

  function toggleFilter(linked: LinkedView) {
    const name = folderName(linked);
    onFilter(isSelected(linked) ? null : name);
    moreOpen = false;
  }

  function unlink(linked: LinkedView) {
    onUnlink({
      sourceId: linkedFolderSourceId(linked) ?? "",
      path: linked.path,
      label: folderName(linked),
    });
  }
</script>

{#snippet folderChip(linked: LinkedView)}
  {@const name = folderName(linked)}
  {@const selected = isSelected(linked)}
  <span
    data-synced-chip
    class="chip !cursor-default border py-1 pr-1 pl-2.5 {selected
      ? 'border-navy-900 bg-navy-900 text-white'
      : 'border-paper-line bg-surface text-ink-soft hover:bg-navy-100/70 dark:hover:bg-white/8'}"
  >
    <button
      type="button"
      class="inline-flex min-w-0 items-center gap-1 rounded-sm text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-500"
      aria-pressed={selected}
      title={linked.available === false ? t("shelves.folderMissing") : linked.path}
      onclick={() => toggleFilter(linked)}
    >
      <FolderSymlink
        size={11.5}
        class="shrink-0 {selected ? 'text-white' : 'text-navy-500'}"
        aria-hidden="true"
      />
      <span class="max-w-[160px] truncate">{name}</span>
      {#if linked.available === false}
        <span class="shrink-0 font-medium {selected ? 'text-white/80' : 'text-ink-faint'}"
          >{t("shelves.unavailable")}</span
        >
      {/if}
    </button>
    <button
      type="button"
      class="rounded p-0.5 hover:bg-red-50 hover:text-red-700 dark:hover:bg-red-400/10 dark:hover:text-red-400"
      aria-label={t("shelves.removeSynced")}
      title={t("shelves.removeSyncedHint")}
      onclick={() => unlink(linked)}
    >
      <X size={11} aria-hidden="true" />
    </button>
  </span>
{/snippet}

<div class="relative min-w-0 flex-1">
  <div bind:this={rowEl} class="flex min-w-0 flex-nowrap items-center justify-end gap-1.5">
    <p data-synced-label class="label shrink-0 !text-[10px] whitespace-nowrap">
      {t("shelves.syncedFolders")}
    </p>
    {#each visibleFolders as linked (linked.path || linked.sourceId)}
      {@render folderChip(linked)}
    {/each}
    {#if hiddenCount > 0}
      <div class="relative shrink-0">
        <button
          type="button"
          class="chip shrink-0 border border-paper-line bg-surface text-ink-soft hover:bg-navy-100/70 dark:hover:bg-white/8"
          aria-label={t("shelves.moreSynced")}
          aria-haspopup="dialog"
          aria-expanded={moreOpen}
          aria-controls="synced-folders-menu"
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
            id="synced-folders-menu"
            class="absolute top-full right-0 z-30 mt-1.5 max-h-64 min-w-[240px] overflow-y-auto rounded-xl border border-paper-line bg-surface p-2 shadow-pop dark:shadow-none"
            role="dialog"
            aria-label={t("shelves.syncedFolders")}
            tabindex="-1"
            use:focusTrap
            onkeydown={(event) => event.key === "Escape" && (moreOpen = false)}
          >
            <p class="label px-1 pb-1.5 !text-[10px]">{t("shelves.syncedFolders")}</p>
            <ul class="flex flex-col gap-1.5">
              {#each folders as linked (linked.path || linked.sourceId)}
                <li class="min-w-0">{@render folderChip(linked)}</li>
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
    <p data-synced-label class="label shrink-0 !text-[10px] whitespace-nowrap">
      {t("shelves.syncedFolders")}
    </p>
    {#each folders as linked (linked.path || linked.sourceId)}
      {@render folderChip(linked)}
    {/each}
    <span data-synced-more class="chip shrink-0 border border-paper-line">...</span>
  </div>
</div>
