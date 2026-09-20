<script lang="ts">
  import { api, type DocumentMeta } from "$lib/api";
  import { importIntoChat } from "$lib/chat-import";
  import { fileListQuery, placeholderAt, replacePlaceholder } from "$lib/placeholders";
  import { shelfDisplayName } from "$lib/shelf-label";
  import {
    app,
    chatState,
    fillDraft,
    loadShelfDocuments,
    logInvokeError,
    notifyInvokeError,
    openCreateShelf,
  } from "$lib/stores.svelte";
  import { focusTrap } from "$lib/focus-trap";
  import { t } from "$lib/i18n.svelte";
  import { PROMPT_MAX_CHARS } from "$lib/text-cap";
  import { SendHorizontal, Square, ChevronDown, LibraryBig, Plus, Paperclip } from "@lucide/svelte";

  let {
    composerEl = $bindable(null),
    hasModel,
    generating,
    onSend,
    onStop,
    onChooseShelf,
    onAutoResize,
  }: {
    composerEl: HTMLTextAreaElement | null;
    hasModel: boolean;
    generating: boolean;
    onSend: () => void;
    onStop: () => void;
    onChooseShelf: (id: string | null) => void;
    onAutoResize: () => void;
  } = $props();

  let shelfMenuOpen = $state(false);
  let cursor = $state(0);
  let dismissedSlot = $state<string | null>(null);
  let highlight = $state(0);

  const selectedShelf = $derived.by(() => {
    const id = chatState.selectedShelfId;
    if (!id) return null;
    if (chatState.uploadShelf?.id === id) return null;
    return app.shelves.find((shelf) => shelf.id === id) ?? null;
  });
  const selectedLabel = $derived(
    selectedShelf ? shelfDisplayName(selectedShelf) : t("chat.noShelf"),
  );
  const filesListLabel = $derived.by(() => {
    if (selectedShelf && chatState.uploadShelf) return t("chat.filesOnShelfAndConversation");
    if (chatState.uploadShelf) return t("chat.filesInConversation");
    return t("chat.filesOnShelf");
  });

  $effect(() => {
    for (const id of [selectedShelf?.id, chatState.uploadShelf?.id]) {
      if (id) void loadShelfDocuments(id).catch((error) => logInvokeError(error, "composer files"));
    }
  });
  const files = $derived([
    ...new Map(
      [
        ...(app.documents[selectedShelf?.id ?? ""] ?? []),
        ...(app.documents[chatState.uploadShelf?.id ?? ""] ?? []),
      ].map((doc) => [doc.id, doc]),
    ).values(),
  ]);

  const uploadDocs = $derived(app.documents[chatState.uploadShelf?.id ?? ""] ?? []);
  const uploadErrors = $derived(uploadDocs.filter((doc) => doc.status === "error"));
  const uploadReading = $derived(
    uploadDocs.filter((doc) => doc.status === "reading").length +
      (chatState.uploadShelf?.stats?.waiting ?? 0),
  );

  const activeSlot = $derived(placeholderAt(chatState.draft, cursor));
  const fileMatches = $derived.by(() => {
    if (!activeSlot || files.length === 0) return [];
    const query = fileListQuery(activeSlot.inner);
    if (query === null) return [];
    const matches = query
      ? files.filter((doc) => `${doc.fileName} ${doc.relPath}`.toLowerCase().includes(query))
      : files;
    return matches.slice(0, 8);
  });
  const slotKey = $derived(activeSlot ? `${activeSlot.start}:${activeSlot.inner}` : null);
  const listOpen = $derived(fileMatches.length > 0 && dismissedSlot !== slotKey);

  $effect(() => {
    void fileMatches.length;
    highlight = 0;
  });

  function syncCursor() {
    if (composerEl) cursor = composerEl.selectionStart ?? 0;
  }

  function pickFile(doc: DocumentMeta) {
    const duplicate = files.filter((file) => file.fileName === doc.fileName).length > 1;
    const samePath = files.filter((file) => file.relPath === doc.relPath).length > 1;
    const name = duplicate ? (samePath ? doc.path : doc.relPath) : doc.fileName;
    if (!activeSlot) return;
    fillDraft(replacePlaceholder(chatState.draft, activeSlot, name));
    dismissedSlot = slotKey;
    composerEl?.focus();
  }

  async function attachFiles() {
    await importIntoChat();
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.isComposing || event.keyCode === 229) return;
    if (listOpen) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        highlight = (highlight + 1) % fileMatches.length;
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        highlight = (highlight - 1 + fileMatches.length) % fileMatches.length;
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        dismissedSlot = slotKey;
        const end = activeSlot?.end ?? cursor;
        cursor = end;
        composerEl?.setSelectionRange(end, end);
        return;
      }
      if (!event.shiftKey && (event.key === "Enter" || event.key === "Tab")) {
        const name = fileMatches[highlight];
        if (name) {
          event.preventDefault();
          pickFile(name);
          return;
        }
      }
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (!generating && hasModel) onSend();
    }
  }
</script>

<div class="px-3 pt-3 pb-4 min-[900px]:px-6">
  <div class="mx-auto max-w-[760px]">
    {#if !hasModel}
      <button
        type="button"
        id="composer-needs-ai"
        class="group mb-2 flex w-full items-center justify-between rounded-lg border border-navy-500/50 bg-navy-100 px-3 py-2 text-[12.5px] text-ink hover:border-navy-500 hover:bg-navy-200/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-800 dark:border-white/15 dark:bg-white/8 dark:hover:border-navy-400 dark:hover:bg-white/12 dark:focus-visible:outline-navy-400"
        onclick={() => (app.view = "settings")}
      >
        <span>{t("chat.needsAi")}</span>
        <span
          class="font-semibold text-navy-700 group-hover:text-navy-900 dark:text-navy-200 dark:group-hover:text-white"
          >{t("chat.install")}</span
        >
      </button>
    {/if}
    {#if chatState.uploadShelf && (uploadDocs.length > 0 || uploadReading > 0)}
      <details
        class="mb-2 rounded-lg border border-paper-line bg-paper-soft px-3 py-2 text-xs text-ink-soft"
        open={uploadErrors.length > 0}
      >
        <summary class="cursor-pointer"
          >{t("chat.filesInConversation")} · {t("chat.fileCount", {
            count: uploadDocs.length,
          })}{uploadReading > 0 ? ` · ${t("documents.reading")}` : ""}{uploadErrors.length > 0
            ? ` · ${t("shelves.error")}`
            : ""}</summary
        >
        <ul class="mt-2 max-h-32 overflow-auto">
          {#each uploadDocs as doc (doc.id)}
            <li class="flex items-center justify-between gap-2 py-1">
              <span class="min-w-0 truncate" title={doc.relPath}>{doc.fileName}</span><span
                >{t(
                  doc.status === "ready"
                    ? "shelves.ready"
                    : doc.status === "reading"
                      ? "documents.reading"
                      : "shelves.error",
                )}</span
              >
            </li>
          {/each}
        </ul>
        {#if uploadErrors.length > 0}<button
            class="btn-ghost mt-1"
            onclick={() => api.shelfRetryFailed(chatState.uploadShelf!.id).catch(notifyInvokeError)}
            >{t("shelves.tryAgain")}</button
          >{/if}
      </details>
    {/if}
    <div class="card relative flex flex-col gap-1 !rounded-2xl px-3 pt-2.5 pb-2">
      {#if listOpen}
        <div
          class="absolute right-0 bottom-full left-0 z-30 mb-1 max-h-56 overflow-y-auto rounded-xl border border-paper-line bg-surface py-1 shadow-pop dark:shadow-none"
          role="listbox"
          id="composer-file-list"
          aria-label={filesListLabel}
        >
          {#each fileMatches as doc, index (doc.id)}
            <button
              type="button"
              id={`composer-file-${doc.id}`}
              tabindex="-1"
              role="option"
              aria-selected={index === highlight}
              class="flex w-full px-3 py-1.5 text-left text-[12.5px] {index === highlight
                ? 'bg-navy-50 font-medium text-navy-800 dark:bg-white/8 dark:text-ink'
                : 'text-ink hover:bg-paper-soft'}"
              onmousedown={(event) => event.preventDefault()}
              onclick={() => pickFile(doc)}
            >
              <span class="min-w-0 truncate"
                >{doc.fileName}<small class="block truncate font-normal text-ink-soft"
                  >{doc.sourceLabel} · {doc.path}</small
                ></span
              >
            </button>
          {/each}
        </div>
      {/if}
      <textarea
        bind:this={composerEl}
        bind:value={chatState.draft}
        oninput={() => {
          dismissedSlot = null;
          syncCursor();
          onAutoResize();
        }}
        onkeydown={onKeydown}
        onkeyup={syncCursor}
        onclick={syncCursor}
        onselect={syncCursor}
        onfocus={() => {
          syncCursor();
          if (hasModel) api.warmEngine().catch((error) => logInvokeError(error, "warm engine"));
        }}
        rows="1"
        maxlength={PROMPT_MAX_CHARS}
        aria-label={t("chat.messageLabel")}
        aria-describedby={!hasModel ? "composer-needs-ai" : undefined}
        aria-autocomplete="list"
        aria-activedescendant={listOpen ? `composer-file-${fileMatches[highlight]?.id}` : undefined}
        aria-controls={listOpen ? "composer-file-list" : undefined}
        placeholder={hasModel ? t("chat.placeholderReady") : t("chat.placeholderNeedsAi")}
        class="max-h-[180px] w-full cursor-text resize-none bg-transparent text-[13.8px] leading-relaxed outline-none select-text placeholder:text-ink-faint"
      ></textarea>
      <div class="flex items-center justify-between gap-2">
        <div class="flex items-center gap-1">
          <div class="relative">
            <button
              type="button"
              class="chip border {selectedShelf
                ? 'border-navy-300 bg-navy-100/70 text-navy-800 dark:border-white/15 dark:bg-white/10 dark:text-navy-100'
                : 'border-paper-line bg-paper-soft text-ink-soft'} hover:border-navy-400"
              onclick={() => (shelfMenuOpen = !shelfMenuOpen)}
              aria-haspopup="dialog"
              aria-expanded={shelfMenuOpen}
              aria-label={t("chat.chooseShelf")}
            >
              <LibraryBig size={12.5} aria-hidden="true" />
              {selectedLabel}
              <ChevronDown size={12} />
            </button>
            {#if shelfMenuOpen}
              <div
                class="fixed inset-0 z-20"
                role="presentation"
                onclick={() => (shelfMenuOpen = false)}
                onkeydown={(e) => e.key === "Escape" && (shelfMenuOpen = false)}
              ></div>
              <div
                class="absolute bottom-8 left-0 z-30 w-56 overflow-hidden rounded-xl border border-paper-line bg-surface shadow-pop dark:shadow-none"
                use:focusTrap
                role="dialog"
                tabindex="-1"
                aria-label={t("chat.chooseShelf")}
                onkeydown={(event) => {
                  if (event.key === "Escape") {
                    shelfMenuOpen = false;
                    event.stopPropagation();
                  }
                }}
              >
                <div class="py-1">
                  <button
                    type="button"
                    class="flex w-full items-center gap-2 px-3 py-2 text-left text-[12.5px] hover:bg-paper-soft {chatState.selectedShelfId ===
                    null
                      ? 'font-semibold text-navy-800 dark:text-ink'
                      : 'text-ink'}"
                    onclick={() => {
                      shelfMenuOpen = false;
                      onChooseShelf(null);
                    }}
                  >
                    {t("chat.noShelf")}
                    <span class="ml-auto text-[10.5px] text-ink-faint"
                      >{t("chat.generalQuestions")}</span
                    >
                  </button>
                  {#each app.shelves as shelf (shelf.id)}
                    <button
                      type="button"
                      class="flex w-full items-center gap-2 px-3 py-2 text-left text-[12.5px] hover:bg-paper-soft {chatState.selectedShelfId ===
                      shelf.id
                        ? 'font-semibold text-navy-800 dark:text-ink'
                        : 'text-ink'}"
                      onclick={() => {
                        shelfMenuOpen = false;
                        onChooseShelf(shelf.id);
                      }}
                    >
                      <LibraryBig size={12.5} class="text-ink-faint" />
                      {shelfDisplayName(shelf)}
                      <span class="ml-auto text-[10.5px] text-ink-faint"
                        >{t("chat.fileCount", { count: shelf.stats.searchable })}</span
                      >
                    </button>
                  {/each}
                </div>
                {#if app.shelves.length === 0}
                  <div class="border-t border-paper-line p-1.5">
                    <button
                      type="button"
                      class="btn-outline w-full !py-1.5 !text-[12.5px]"
                      onclick={() => {
                        shelfMenuOpen = false;
                        openCreateShelf();
                      }}
                    >
                      <Plus size={13} />
                      {t("chat.newShelf")}
                    </button>
                  </div>
                {/if}
              </div>
            {/if}
          </div>
          <button
            type="button"
            class="btn-ghost !rounded-full !p-2"
            onclick={attachFiles}
            title={t("chat.addFiles")}
            aria-label={t("chat.addFiles")}
          >
            <Paperclip size={15} aria-hidden="true" />
          </button>
        </div>
        {#if generating}
          <button
            type="button"
            class="btn-primary btn-icon"
            onclick={onStop}
            title={t("chat.stop")}
            aria-label={t("chat.stopGenerating")}
          >
            <Square size={14} fill="currentColor" />
          </button>
        {:else}
          <button
            type="button"
            class="btn-amber btn-icon"
            onclick={onSend}
            disabled={!hasModel || !chatState.draft.trim()}
            title={hasModel ? t("chat.send") : t("chat.installFirstTitle")}
            aria-label={hasModel ? t("chat.sendMessage") : t("chat.installFirstTitle")}
            aria-describedby={!hasModel ? "composer-needs-ai" : undefined}
          >
            <SendHorizontal size={15} />
          </button>
        {/if}
      </div>
    </div>
  </div>
</div>
