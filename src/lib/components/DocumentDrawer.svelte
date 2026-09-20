<script lang="ts">
  import {
    api,
    formatBytes,
    formatWhen,
    piiEmptyHint,
    piiLabel,
    type Card,
    type DocumentMeta,
    type DocumentTextWindow,
  } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import CopyActions from "$lib/components/CopyActions.svelte";
  import Markdown from "$lib/components/Markdown.svelte";
  import { formatCardSummary } from "$lib/markdown";
  import { notifyInvokeError } from "$lib/stores.svelte";
  import { drawerPanel, overlay } from "$lib/motion";
  import { focusTrap } from "$lib/focus-trap";
  import { FolderOpen, FileText, RefreshCw, ScanText, X } from "@lucide/svelte";

  let {
    doc,
    card,
    extractedText = $bindable(null),
    onClose,
  }: {
    doc: DocumentMeta;
    card: Card | null;
    extractedText: string | null;
    onClose: () => void;
  } = $props();

  function statusLabel(document: DocumentMeta): string {
    if (document.status === "ready") return t("documents.ready");
    if (document.status === "reading") return t("documents.reading");
    return document.error ?? t("documents.couldntRead");
  }

  let excerpt = $state<DocumentTextWindow | null>(null);
  let paging = $state(false);
  let request = 0;
  $effect(() => {
    void doc.id;
    request += 1;
    paging = false;
    excerpt = null;
    return () => {
      request += 1;
    };
  });
  let scrollEl = $state<HTMLDivElement | null>(null);

  $effect(() => {
    if (extractedText === null) excerpt = null;
  });

  async function viewExtracted(startChar?: number) {
    const load = ++request;
    const selected = doc;
    paging = true;
    try {
      const window = await api.documentText(selected.shelfId, selected.id, { startChar });
      if (load !== request) return;
      excerpt = window;
      extractedText = excerpt.text;
      scrollEl?.scrollTo(0, 0);
    } catch (error) {
      notifyInvokeError(error);
    } finally {
      if (load === request) paging = false;
    }
  }

  const hasBefore = $derived(!!excerpt && excerpt.startChar > 0);
  const hasAfter = $derived(!!excerpt && excerpt.endChar < excerpt.totalChars);
</script>

<div
  class="fixed inset-0 z-40 flex items-stretch justify-end bg-navy-950/25 dark:bg-black/50"
  role="dialog"
  aria-modal="true"
  aria-label={doc.fileName}
  tabindex="-1"
  use:focusTrap
  transition:overlay
  onclick={(e) => e.target === e.currentTarget && onClose()}
  onkeydown={(e) => e.key === "Escape" && onClose()}
>
  <div
    class="flex h-full w-full max-w-[480px] flex-col overflow-hidden bg-surface shadow-pop dark:shadow-none"
    in:drawerPanel
  >
    <div class="border-b border-paper-line bg-paper-soft px-5 py-4">
      <div class="flex items-start justify-between gap-3">
        <div class="flex min-w-0 items-start gap-3">
          <span
            class="mt-0.5 rounded-lg bg-navy-100 p-2 text-navy-700 dark:bg-white/10 dark:text-navy-200"
            ><FileText size={16} /></span
          >
          <div class="min-w-0">
            <p class="truncate text-[14.5px] font-semibold text-ink" title={doc.fileName}>
              {doc.fileName}
            </p>
            <p
              class="mt-0.5 text-[12px] {doc.status === 'error'
                ? 'text-red-700/80 dark:text-red-400'
                : doc.status === 'ready'
                  ? 'text-ready-ink dark:text-navy-300'
                  : 'text-amber-550'}"
            >
              {statusLabel(doc)}
            </p>
          </div>
        </div>
        <button
          type="button"
          class="btn-ghost !p-1.5"
          aria-label={t("documents.close")}
          onclick={onClose}><X size={15} /></button
        >
      </div>
    </div>

    <div bind:this={scrollEl} class="min-h-0 flex-1 overflow-y-auto px-5 py-4">
      {#if extractedText !== null}
        <div class="mb-3 flex items-center justify-between gap-2">
          <span class="label">{t("documents.textAsRead")}</span>
          <button
            type="button"
            class="btn-ghost !py-1 !text-[11.5px]"
            onclick={() => (extractedText = null)}>{t("documents.backToDetails")}</button
          >
        </div>
        {#if hasBefore || hasAfter}
          <div class="mb-2 flex items-center justify-between gap-2">
            <p class="text-[12px] text-ink-soft">{t("documents.partOfFile")}</p>
            <div class="flex gap-1">
              <button
                type="button"
                class="btn-ghost !py-1 !text-[11.5px]"
                disabled={!hasBefore || paging}
                onclick={() =>
                  excerpt && viewExtracted(Math.max(0, excerpt.startChar - excerpt.windowChars))}
              >
                {t("documents.earlier")}
              </button>
              <button
                type="button"
                class="btn-ghost !py-1 !text-[11.5px]"
                disabled={!hasAfter || paging}
                onclick={() => excerpt && viewExtracted(excerpt.endChar)}
              >
                {t("documents.later")}
              </button>
            </div>
          </div>
        {/if}
        <pre
          class="rounded-lg border border-paper-line bg-paper-soft p-3 text-[11.5px] leading-relaxed whitespace-pre-wrap">{extractedText}</pre>
      {:else}
        <dl class="grid grid-cols-2 gap-x-4 gap-y-2.5 text-[12.5px]">
          <div>
            <dt class="label">{t("documents.source")}</dt>
            <dd class="mt-0.5 text-ink">
              {doc.sourceType === "imported"
                ? t("shelves.imported")
                : t("documents.linked", { label: doc.sourceLabel })}
            </dd>
          </div>
          <div>
            <dt class="label">{t("documents.size")}</dt>
            <dd class="mt-0.5 text-ink">{formatBytes(doc.sizeBytes)}</dd>
          </div>
          {#if doc.pages}<div>
              <dt class="label">{t("documents.pages")}</dt>
              <dd class="mt-0.5 text-ink">{doc.pages}</dd>
            </div>{/if}
          <div>
            <dt class="label">{t("documents.passages")}</dt>
            <dd class="mt-0.5 text-ink">{doc.passageCount}</dd>
          </div>
          {#if card?.language}<div>
              <dt class="label">{t("documents.language")}</dt>
              <dd class="mt-0.5 text-ink uppercase">{card.language}</dd>
            </div>{/if}
          <div>
            <dt class="label">{t("documents.updated")}</dt>
            <dd class="mt-0.5 text-ink">{formatWhen(doc.updatedAt)}</dd>
          </div>
        </dl>

        {#if doc.ocr}
          <p
            class="mt-3 flex items-center gap-2 rounded-lg bg-navy-50 px-3 py-2 text-[12px] text-navy-800 dark:bg-white/8 dark:text-navy-100"
          >
            <ScanText size={13.5} />
            {t("documents.ocrHint")}
          </p>
        {/if}

        {#if card}
          {#if card.summary}
            <h3 class="label mt-5 mb-1.5">{t("documents.summary")}</h3>
            <Markdown
              text={formatCardSummary(
                card.summary,
                card.outline.map((entry) => entry.title),
              )}
              compact
            />
          {/if}
          {#if card.keywords.length > 0}
            <h3 class="label mt-4 mb-1.5">{t("documents.keywords")}</h3>
            <div class="flex flex-wrap gap-1.5">
              {#each card.keywords as keyword}
                <span class="chip !cursor-default bg-paper-soft text-ink-soft">{keyword}</span>
              {/each}
            </div>
          {/if}
          {#if card.outline.length > 0}
            <h3 class="label mt-4 mb-1.5">{t("documents.outline")}</h3>
            <ul class="space-y-1">
              {#each card.outline.slice(0, 14) as entry}
                <li class="flex items-baseline justify-between gap-3 text-[12.5px]">
                  <span class="truncate text-ink">{entry.title}</span>
                  {#if entry.page}<span class="shrink-0 text-ink-faint"
                      >{t("documents.pageAbbrev", { page: entry.page })}</span
                    >{/if}
                </li>
              {/each}
            </ul>
          {/if}
        {/if}

        {#if doc.piiTotal > 0}
          <h3 class="label mt-5 mb-1.5">{t("pii.heading")}</h3>
          <div class="rounded-lg border border-paper-line bg-paper-soft/60 px-3.5 py-2.5">
            {#each Object.entries(doc.piiCategories ?? {}) as [category, count]}
              <p class="flex justify-between py-0.5 text-[12.5px]">
                <span class="text-ink-soft">{piiLabel(category, count)}</span>
                <span class="font-semibold text-ink tabular-nums">{count}</span>
              </p>
            {/each}
          </div>
        {:else if doc.status === "ready"}
          <h3 class="label mt-5 mb-1.5">{t("pii.heading")}</h3>
          <p class="text-[12px] text-ink-faint">{piiEmptyHint()}</p>
        {/if}
      {/if}
    </div>

    <div class="flex flex-col gap-2 border-t border-paper-line px-4 py-3">
      {#if extractedText !== null}
        <CopyActions text={extractedText} />
      {/if}
      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class="btn-outline shrink-0 !text-[12px] whitespace-nowrap"
          onclick={() => api.openOriginal(doc.path).catch(notifyInvokeError)}
        >
          <FileText size={13} class="shrink-0" aria-hidden="true" />
          {t("documents.openOriginal")}
        </button>
        <button
          type="button"
          class="btn-outline shrink-0 !text-[12px] whitespace-nowrap"
          onclick={() => api.revealItem(doc.path).catch(notifyInvokeError)}
        >
          <FolderOpen size={13} class="shrink-0" aria-hidden="true" />
          {t("documents.showInFolder")}
        </button>
        {#if doc.status === "ready" && extractedText === null}
          <button
            type="button"
            class="btn-outline shrink-0 !text-[12px] whitespace-nowrap"
            onclick={() => viewExtracted()}
          >
            <ScanText size={13} class="shrink-0" aria-hidden="true" />
            {t("documents.viewText")}
          </button>
        {/if}
        {#if doc.status === "error"}
          <button
            type="button"
            class="btn-amber shrink-0 !text-[12px] whitespace-nowrap"
            onclick={() =>
              api
                .documentReprocess(doc.shelfId, doc.id)
                .then(() => onClose())
                .catch(notifyInvokeError)}
          >
            <RefreshCw size={13} class="shrink-0" aria-hidden="true" />
            {t("shelves.tryAgain")}
          </button>
        {/if}
      </div>
    </div>
  </div>
</div>
