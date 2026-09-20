<script lang="ts">
  import { fileTypeLabel, formatWhen, type DocumentMeta } from "$lib/api";
  import { tick } from "svelte";
  import { app } from "$lib/stores.svelte";
  import { virtualWindow } from "$lib/virtual-window";
  import { t } from "$lib/i18n.svelte";

  let {
    visibleDocs,
    onOpen,
    onRetry,
    onClear,
  }: {
    visibleDocs: DocumentMeta[];
    onOpen: (doc: DocumentMeta) => void;
    onRetry: (doc: DocumentMeta) => void;
    onClear: () => void;
  } = $props();

  let viewport = $state<HTMLDivElement | null>(null);
  let top = $state(0);
  let height = $state(480);
  let rowHeight = $state(48);
  let showAll = $state(false);
  const virtual = $derived(visibleDocs.length > 100 && !showAll);
  const window = $derived(
    virtual
      ? virtualWindow(visibleDocs.length, top, height, rowHeight)
      : { start: 0, end: visibleDocs.length, before: 0, after: 0 },
  );
  const rows = $derived(visibleDocs.slice(window.start, window.end));
  $effect(() => {
    void app.settings?.textSize;
    if (!viewport) return;
    const node = viewport;
    const measure = () => {
      height = node.clientHeight;
      const cell = node.querySelector("td button");
      rowHeight = cell
        ? Math.max(48, Math.ceil(parseFloat(getComputedStyle(cell).lineHeight) + 28))
        : 48;
    };
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    void tick().then(measure);
    return () => observer.disconnect();
  });
  async function rowKey(event: KeyboardEvent, index: number) {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    const target = Math.max(
      0,
      Math.min(
        visibleDocs.length - 1,
        event.key === "Home"
          ? 0
          : event.key === "End"
            ? visibleDocs.length - 1
            : index + (event.key === "ArrowDown" ? 1 : -1),
      ),
    );
    if (viewport && virtual) {
      viewport.scrollTop = target * rowHeight;
      top = viewport.scrollTop;
    }
    await tick();
    viewport?.querySelector<HTMLButtonElement>(`[data-row="${target}"]`)?.focus();
  }

  const columns = $derived([
    { header: t("shelves.colFile"), colClass: "", thClass: "pr-2.5 pl-8" },
    { header: t("shelves.colSource"), colClass: "w-44", thClass: "px-2.5" },
    { header: t("shelves.colType"), colClass: "w-28", thClass: "px-2.5" },
    { header: t("shelves.colStatus"), colClass: "w-44", thClass: "px-2.5" },
    { header: t("shelves.colPii"), colClass: "w-14", thClass: "px-2.5 text-right" },
    { header: t("shelves.colUpdated"), colClass: "w-36", thClass: "pr-8 pl-2.5" },
  ]);
</script>

{#if visibleDocs.length === 0}
  <div class="flex flex-col items-center justify-center gap-4 px-8 py-16 text-center">
    <div class="flex flex-col items-center gap-1">
      <p class="text-sm font-medium text-ink">{t("shelves.noMatch")}</p>
      <p class="max-w-[40ch] text-[13px] text-pretty text-ink-soft">
        {t("shelves.tryDifferentFilter")}
      </p>
    </div>
    <button type="button" class="btn-ghost" onclick={onClear}>{t("shelves.clearFilters")}</button>
  </div>
{:else}
  {#if visibleDocs.length > 100}
    <label class="mx-8 my-2 flex items-center gap-2 text-xs text-ink-soft"
      ><input type="checkbox" bind:checked={showAll} />{t("shelves.showAllRows")}</label
    >
  {/if}
  <div
    bind:this={viewport}
    class="max-h-[60vh] overflow-auto"
    onscroll={() => {
      top = viewport?.scrollTop ?? 0;
    }}
  >
    <table
      aria-rowcount={visibleDocs.length + 1}
      class="w-full min-w-[850px] table-fixed border-separate border-spacing-0 text-[12.8px]"
    >
      <colgroup>
        {#each columns as column}
          <col class={column.colClass} />
        {/each}
      </colgroup>
      <thead>
        <tr class="text-left">
          {#each columns as column}
            <th
              scope="col"
              class="sticky top-0 z-10 border-b border-paper-line bg-paper py-2 font-medium whitespace-nowrap text-ink-faint {column.thClass}"
              >{column.header}</th
            >
          {/each}
        </tr>
      </thead>
      <tbody>
        {#if window.before > 0}<tr aria-hidden="true"
            ><td colspan="6" style={`height:${window.before}px; padding:0; border:0`}></td></tr
          >{/if}
        {#each rows as doc, offset (doc.id)}
          <tr
            aria-rowindex={window.start + offset + 2}
            style={`height:${rowHeight}px`}
            class="cursor-pointer whitespace-nowrap hover:bg-navy-50/60 dark:hover:bg-white/4"
            onclick={() => onOpen(doc)}
          >
            <td
              class="truncate border-b border-paper-line/70 py-2.5 pr-2.5 pl-8 font-medium text-ink"
              title={doc.relPath}
            >
              <button
                type="button"
                data-row={window.start + offset}
                onkeydown={(event) => rowKey(event, window.start + offset)}
                class="block w-full truncate text-left font-medium text-ink focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-navy-500"
                onclick={(event) => {
                  event.stopPropagation();
                  onOpen(doc);
                }}>{doc.fileName}</button
              >
            </td>
            <td
              class="truncate border-b border-paper-line/70 px-2.5 py-2.5 text-ink-soft"
              title={doc.sourceType === "imported" ? t("shelves.imported") : doc.sourceLabel}
            >
              {doc.sourceType === "imported"
                ? t("shelves.imported")
                : t("documents.linked", { label: doc.sourceLabel })}
            </td>
            <td class="truncate border-b border-paper-line/70 px-2.5 py-2.5 text-ink-soft"
              >{fileTypeLabel(doc)}</td
            >
            <td class="border-b border-paper-line/70 px-2.5 py-2.5">
              {#if doc.status === "ready"}
                <span class="text-ready-ink dark:text-navy-300">{t("shelves.ready")}</span>
              {:else if doc.status === "reading"}
                <span class="inline-flex items-center gap-1.5 text-amber-550">
                  <span
                    class="inline-block size-1.5 animate-pulse rounded-full bg-amber-450"
                    aria-hidden="true"
                  ></span>
                  {t("documents.reading")}
                </span>
              {:else}
                <span class="inline-flex items-center gap-2">
                  <span class="text-red-700/80 dark:text-red-400" title={doc.error}
                    >{t("shelves.error")}</span
                  >
                  <button
                    type="button"
                    class="btn-ghost !px-1.5 !py-0.5 !text-[11.5px]"
                    aria-label={t("shelves.tryAgainFile", { name: doc.fileName })}
                    onclick={(event) => {
                      event.stopPropagation();
                      onRetry(doc);
                    }}>{t("shelves.tryAgain")}</button
                  >
                </span>
              {/if}
            </td>
            <td
              class="border-b border-paper-line/70 px-2.5 py-2.5 text-right text-ink-soft tabular-nums"
            >
              {doc.piiTotal > 0 ? doc.piiTotal : "—"}
            </td>
            <td class="truncate border-b border-paper-line/70 py-2.5 pr-8 pl-2.5 text-ink-soft"
              >{formatWhen(doc.updatedAt)}</td
            >
          </tr>
        {/each}
        {#if window.after > 0}<tr aria-hidden="true"
            ><td colspan="6" style={`height:${window.after}px; padding:0; border:0`}></td></tr
          >{/if}
      </tbody>
    </table>
  </div>
{/if}
