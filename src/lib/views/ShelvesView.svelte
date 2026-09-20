<script lang="ts">
  import { api, type Card, type DocumentMeta, type LinkedView, type ShelfView } from "$lib/api";
  import {
    app,
    chatState,
    notify,
    notifyInvokeError,
    loadShelfDocuments,
    rememberPreferredShelf,
    refreshShelves,
    refreshThreads,
  } from "$lib/stores.svelte";
  import { linkedFolderName, linkedFolderSourceId, listenFileDrop } from "$lib/files";
  import {
    LibraryBig,
    Plus,
    FolderSymlink,
    FolderInput,
    FolderPlus,
    X,
    Trash2,
    ShieldCheck,
    ChevronLeft,
    Search,
    RefreshCw,
    Pencil,
  } from "@lucide/svelte";
  import PiiCategoriesRow from "$lib/components/PiiCategoriesRow.svelte";
  import ShelfDocumentTable from "$lib/components/ShelfDocumentTable.svelte";
  import SyncedFoldersRow from "$lib/components/SyncedFoldersRow.svelte";
  import ThinkLevelSlider from "$lib/components/ThinkLevelSlider.svelte";
  import DocumentDrawer from "$lib/components/DocumentDrawer.svelte";
  import { confirmDanger } from "$lib/native-dialog";
  import { t } from "$lib/i18n.svelte";
  import { shot } from "$lib/shot-control.svelte";
  import { importFeedback } from "$lib/shelf-cap";
  import { shelfListStatus, shelfListStatusClass, shelfListStatusLabel } from "$lib/shelf-status";

  let creating = $state(app.createShelf || import.meta.env.VITE_START_CREATE_SHELF === "1");
  if (app.createShelf) app.createShelf = false;

  let newName = $state("");
  const documents = $derived(app.documents[app.openShelfId ?? ""] ?? []);
  let dropActive = $state(false);
  let filterPii = $state<string | null>(null);
  let filterSource = $state<string | null>(null);
  let filterType = $state<string | null>(null);
  let searchText = $state("");
  let openDoc = $state<DocumentMeta | null>(null);
  let openedStartDoc = $state(false);
  let openCard = $state<Card | null>(null);
  let extractedText = $state<string | null>(null);
  let deleting = $state(false);
  let renamingId = $state<string | null>(null);
  let renameName = $state("");

  const shelf = $derived(app.shelves.find((s) => s.id === app.openShelfId) ?? null);

  // Opening the list runs the stale-Reading check in shelves_list.
  $effect(() => {
    if (app.openShelfId === null) {
      void refreshShelves();
    }
  });

  async function loadDocuments(shelfId: string) {
    try {
      await loadShelfDocuments(shelfId, true);
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  // Refetch when the open shelf or ingest tick changes.
  $effect(() => {
    const id = app.openShelfId;
    if (id) {
      void loadShelfDocuments(id).catch(notifyInvokeError);
    }
  });

  $effect(() => {
    if (openedStartDoc) return;
    if (!shot.docFirst && import.meta.env.VITE_START_DOC !== "first") return;
    const first = documents[0];
    if (!first) return;
    openedStartDoc = true;
    // Give the table a moment to paint before the drawer (screenshot runner).
    window.setTimeout(() => {
      void openFile(first);
    }, 800);
  });

  // Native drag & drop of files onto the shelf detail.
  $effect(() => {
    if (!shelf) return;
    const shelfId = shelf.id;
    return listenFileDrop({
      onOver: (active) => (dropActive = active),
      onDrop: () => {
        api
          .shelfImportPaths(shelfId, [])
          .then((result) => {
            const message = importFeedback(result.queued, result.atLimit, result.skippedLong ?? 0);
            if (message) notify(message);
          })
          .catch(notifyInvokeError);
      },
    });
  });

  const fileTypes = $derived([...new Set(documents.map((d) => d.format.toUpperCase()))].sort());
  const sourceLabels = $derived(
    [
      ...new Set([
        ...documents.map((d) => d.sourceLabel),
        ...(shelf?.linkedFolders.map((linked) => linkedFolderName(linked)) ?? []),
      ]),
    ].sort(),
  );

  const visibleDocs = $derived(
    documents.filter((doc) => {
      if (filterPii === "any" && doc.piiTotal === 0) return false;
      if (filterPii && filterPii !== "any" && !(doc.piiCategories?.[filterPii] ?? 0)) return false;
      if (filterSource && doc.sourceLabel !== filterSource) return false;
      if (filterType && doc.format.toUpperCase() !== filterType) return false;
      if (searchText && !doc.fileName.toLowerCase().includes(searchText.toLowerCase()))
        return false;
      return true;
    }),
  );

  function beginRename(shelfView: ShelfView, event: MouseEvent) {
    event.stopPropagation();
    renamingId = shelfView.id;
    renameName = shelfView.name;
  }

  async function commitRename(shelfId: string) {
    const name = renameName.trim();
    const current = app.shelves.find((s) => s.id === shelfId)?.name;
    renamingId = null;
    if (!name || name === current) return;
    try {
      await api.shelfRename(shelfId, name);
      await refreshShelves();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  function onRenameKey(event: KeyboardEvent, shelfId: string) {
    if (event.key === "Enter") {
      event.preventDefault();
      void commitRename(shelfId);
    } else if (event.key === "Escape") {
      event.preventDefault();
      renamingId = null;
    }
  }

  async function createShelf() {
    const name = newName.trim();
    if (!name) return;
    try {
      const created = await api.shelfCreate(name);
      newName = "";
      creating = false;
      rememberPreferredShelf(created.id);
      await refreshShelves();
      app.openShelfId = created.id;
      if (chatState.messages.length === 0) {
        chatState.selectedShelfId = created.id;
        if (chatState.activeThreadId) {
          await api.threadSetShelf(chatState.activeThreadId, created.id);
          await refreshThreads();
        }
      }
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  let fileRequest = 0;
  async function openFile(doc: DocumentMeta) {
    const request = ++fileRequest;
    openDoc = doc;
    openCard = null;
    extractedText = null;
    if (doc.status === "ready") {
      try {
        const card = await api.documentCard(doc.shelfId, doc.id);
        if (request === fileRequest && openDoc?.id === doc.id) openCard = card;
      } catch {
        if (request === fileRequest && openDoc?.id === doc.id) openCard = null;
      }
    }
  }

  async function unlinkSource(shelfView: ShelfView, linked: LinkedView) {
    const path = linked.path;
    const label = linkedFolderName(linked);
    const sourceId = linkedFolderSourceId(linked);
    const ok = await confirmDanger(
      t("shelves.removeFolder", { name: label }),
      t("shelves.removeFolderTitle"),
    );
    if (!ok) return;
    try {
      await api.shelfRemoveSource(shelfView.id, { sourceId, path });
      if (filterSource === label) filterSource = null;
      if (sourceId && openDoc?.sourceId === sourceId) openDoc = null;
      app.documents[shelfView.id] = (app.documents[shelfView.id] ?? []).filter((doc) => {
        if (sourceId && doc.sourceId === sourceId) return false;
        if (doc.sourceType === "linked" && doc.sourceLabel === label) return false;
        return true;
      });
      await refreshShelves();
      if (app.openShelfId === shelfView.id) await loadDocuments(shelfView.id);
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function requestDelete(shelfView: ShelfView, event: MouseEvent) {
    event.stopPropagation();
    if (deleting) return;
    const ok = await confirmDanger(
      t("shelves.deleteShelfConfirm", { name: shelfView.name }),
      t("shelves.deleteShelfAction"),
    );
    if (!ok) return;
    deleting = true;
    try {
      await api.shelfRemove(shelfView.id);
      if (app.openShelfId === shelfView.id) app.openShelfId = null;
      await refreshShelves();
    } catch (error) {
      notifyInvokeError(error);
    } finally {
      deleting = false;
    }
  }

  async function retryFile(doc: DocumentMeta) {
    try {
      await api.documentReprocess(doc.shelfId, doc.id);
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function resumeShelf(shelfView: ShelfView, event: MouseEvent) {
    event.stopPropagation();
    try {
      await api.shelfRetryFailed(shelfView.id);
      await refreshShelves();
      if (app.openShelfId === shelfView.id) await loadDocuments(shelfView.id);
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function addFiles() {
    if (!shelf) return;
    try {
      const result = await api.shelfImportDialog(shelf.id);
      if (result.cancelled) return;
      const message = importFeedback(result.queued, result.atLimit, result.skippedLong ?? 0);
      if (message) notify(message);
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function addFolder() {
    if (!shelf) return;
    try {
      const result = await api.shelfAddLinked(shelf.id);
      if (!result) return;
      await refreshShelves();
      const message = importFeedback(result.queued, result.atLimit);
      if (message) notify(message);
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  function clearFilters() {
    filterPii = null;
    filterSource = null;
    filterType = null;
    searchText = "";
  }

  const filtersOn = $derived(!!(filterPii || filterSource || filterType || searchText));
</script>

{#if !shelf}
  <div class="h-full overflow-y-auto">
    <div class="mx-auto max-w-[860px] px-4 py-5 min-[900px]:px-8 min-[900px]:py-8">
      <div class="mb-6 flex flex-wrap items-end justify-between gap-4">
        <div class="min-w-0 flex-1 basis-64">
          <h1 class="text-[22px] font-semibold text-ink">{t("shelves.title")}</h1>
          <p class="mt-0.5 text-[13px] text-ink-soft">
            {t("shelves.lede")}
          </p>
        </div>
        <button type="button" class="btn-primary" onclick={() => (creating = true)}>
          <Plus size={15} />
          {t("shelves.newShelf")}
        </button>
      </div>

      {#if creating}
        <div class="card mb-5 flex items-center gap-2 px-4 py-3">
          <LibraryBig size={16} class="text-navy-500" />
          <label class="sr-only" for="new-shelf-name">{t("shelves.shelfName")}</label>
          <!-- svelte-ignore a11y_autofocus -->
          <input
            id="new-shelf-name"
            name="shelf-name"
            class="min-w-0 flex-1 cursor-text rounded-none border-none bg-transparent text-[14px] text-ink outline-none select-text placeholder:text-ink-faint"
            placeholder={t("shelves.namePlaceholder")}
            bind:value={newName}
            autofocus
            onkeydown={(e) => e.key === "Enter" && createShelf()}
          />
          <button type="button" class="btn-amber" onclick={createShelf}
            >{t("shelves.create")}</button
          >
          <button type="button" class="btn-ghost !py-1.5" onclick={() => (creating = false)}
            >{t("shelves.cancel")}</button
          >
        </div>
      {/if}

      {#if app.shelves.length === 0 && !creating}
        <div class="card flex flex-col items-center px-8 py-14 text-center">
          <div
            class="mb-3 rounded-2xl bg-navy-100 p-3.5 text-navy-700 dark:bg-white/10 dark:text-navy-200"
          >
            <LibraryBig size={24} />
          </div>
          <h2 class="text-[16px] font-semibold text-ink">{t("shelves.emptyTitle")}</h2>
          <p class="mt-1 mb-5 max-w-md text-[13px] leading-relaxed text-ink-soft">
            {t("shelves.emptyBody")}
          </p>
          <button type="button" class="btn-primary" onclick={() => (creating = true)}
            ><Plus size={15} /> {t("shelves.createFirst")}</button
          >
        </div>
      {:else}
        <div class="grid grid-cols-1 gap-4 min-[900px]:grid-cols-2">
          {#each app.shelves as shelfCard (shelfCard.id)}
            {@const status = shelfListStatus(shelfCard.stats)}
            <div class="card group flex items-stretch hover:shadow-pop dark:hover:shadow-none">
              {#if renamingId === shelfCard.id}
                <div class="flex min-w-0 flex-1 items-center gap-3 px-5 py-4">
                  <span class="rounded-xl bg-navy-900 p-2.5 text-mint"
                    ><LibraryBig size={18} aria-hidden="true" /></span
                  >
                  <label class="sr-only" for="rename-shelf-{shelfCard.id}"
                    >{t("shelves.shelfName")}</label
                  >
                  <!-- svelte-ignore a11y_autofocus -->
                  <input
                    id="rename-shelf-{shelfCard.id}"
                    name="shelf-name"
                    class="min-w-0 flex-1 rounded-md border border-navy-300 bg-surface px-2 py-1 text-[15px] font-semibold text-ink outline-none"
                    bind:value={renameName}
                    autofocus
                    onblur={() => void commitRename(shelfCard.id)}
                    onkeydown={(e) => onRenameKey(e, shelfCard.id)}
                  />
                </div>
              {:else}
                <button
                  type="button"
                  class="min-w-0 flex-1 px-5 py-4 text-left"
                  onclick={() => (app.openShelfId = shelfCard.id)}
                >
                  <div class="flex items-center gap-3">
                    <span class="rounded-xl bg-navy-900 p-2.5 text-mint"
                      ><LibraryBig size={18} aria-hidden="true" /></span
                    >
                    <span class="min-w-0">
                      <span class="block truncate text-[15px] font-semibold text-ink"
                        >{shelfCard.name}</span
                      >
                      <span class="mt-0.5 flex flex-wrap items-center gap-2">
                        <span class="text-[12px] text-ink-soft"
                          >{t("shelves.fileCount", { count: shelfCard.stats.files })}</span
                        >
                        {#if status}
                          <span
                            class="chip !cursor-default !px-2 !py-0.5 !text-[11px] {shelfListStatusClass(
                              status,
                            )}"
                          >
                            {#if status === "processing" || status === "syncing"}
                              <span
                                class="inline-block size-1.5 animate-pulse rounded-full bg-amber-450"
                                aria-hidden="true"
                              ></span>
                            {/if}
                            {shelfListStatusLabel(status)}
                          </span>
                        {/if}
                      </span>
                    </span>
                  </div>
                  {#if shelfCard.stats.pii.total > 0}
                    <p class="mt-3 flex items-center gap-1.5 text-[11.5px] text-ink-faint">
                      <ShieldCheck size={12.5} class="text-navy-500" aria-hidden="true" />
                      {t("shelves.filesWithPii", { count: shelfCard.stats.filesWithPii })}
                    </p>
                  {/if}
                </button>
              {/if}
              {#if renamingId !== shelfCard.id}
                <div class="m-3 flex shrink-0 flex-col items-end gap-1.5 self-start">
                  {#if status === "error"}
                    <button
                      type="button"
                      class="btn-outline py-1.5 pr-2.5 pl-1.5 !text-[12px]"
                      onclick={(e) => resumeShelf(shelfCard, e)}
                    >
                      <RefreshCw size={13} class="shrink-0" aria-hidden="true" />
                      {t("shelves.resume")}
                    </button>
                  {/if}
                  <button
                    type="button"
                    class="btn-ghost !p-1.5"
                    aria-label={t("shelves.renameShelf")}
                    title={t("shelves.rename")}
                    onclick={(e) => beginRename(shelfCard, e)}
                  >
                    <Pencil size={14} aria-hidden="true" />
                  </button>
                  <button
                    type="button"
                    class="btn-ghost !p-1.5"
                    aria-label={t("shelves.deleteShelf")}
                    disabled={deleting}
                    onclick={(e) => requestDelete(shelfCard, e)}
                  >
                    <Trash2 size={14} aria-hidden="true" />
                  </button>
                </div>
              {/if}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  </div>
{:else}
  <div class="relative flex h-full min-h-0 flex-col overflow-hidden">
    {#if dropActive && documents.length > 0}
      <div
        class="pointer-events-none absolute inset-3 z-30 flex items-center justify-center rounded-2xl border-2 border-dashed border-navy-500 bg-navy-100/50 dark:bg-white/10"
      >
        <p class="rounded-xl bg-navy-900 px-4 py-2 text-[13.5px] font-medium text-white shadow-pop">
          {t("shelves.dropToAdd", { name: shelf.name })}
        </p>
      </div>
    {/if}

    <header class="flex shrink-0 flex-col gap-3 px-4 pt-5 pb-5 min-[900px]:px-8">
      <button
        type="button"
        class="inline-flex min-h-8 w-fit items-center gap-1 rounded-md text-[13px] font-medium whitespace-nowrap text-ink-soft hover:text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-500"
        onclick={() => (app.openShelfId = null)}
      >
        <ChevronLeft size={16} class="-ml-0.5 shrink-0" aria-hidden="true" />
        {t("shelves.allShelves")}
      </button>

      <div class="flex flex-wrap items-start justify-between gap-4">
        <div class="min-w-0">
          {#if renamingId === shelf.id}
            <label class="sr-only" for="rename-shelf-detail">{t("shelves.shelfName")}</label>
            <!-- svelte-ignore a11y_autofocus -->
            <input
              id="rename-shelf-detail"
              name="shelf-name"
              class="w-full max-w-md rounded-md border border-navy-300 bg-surface px-2 py-1 text-[1.375rem] font-semibold tracking-tight text-ink outline-none"
              bind:value={renameName}
              autofocus
              onblur={() => void commitRename(shelf.id)}
              onkeydown={(e) => onRenameKey(e, shelf.id)}
            />
          {:else}
            <div class="flex min-w-0 items-center gap-1.5">
              <h1 class="truncate text-[1.375rem] font-semibold tracking-tight text-ink">
                {shelf.name}
              </h1>
              <button
                type="button"
                class="btn-ghost shrink-0 !p-1.5"
                aria-label={t("shelves.renameShelf")}
                title={t("shelves.rename")}
                onclick={(e) => beginRename(shelf, e)}
              >
                <Pencil size={14} aria-hidden="true" />
              </button>
            </div>
          {/if}
          <div class="mt-3 w-full max-w-sm min-w-[220px]">
            <ThinkLevelSlider shelfId={shelf.id} value={shelf.thinkLevel ?? "off"} />
          </div>
        </div>

        {#if documents.length > 0}
          <div class="flex shrink-0 flex-wrap items-center justify-end gap-2">
            <button type="button" class="btn-primary" onclick={addFiles}>
              <FolderInput size={14} class="shrink-0" aria-hidden="true" />
              {t("shelves.addFiles")}
            </button>
            <button
              type="button"
              class="btn-outline py-2 pr-3 pl-2"
              title={t("shelves.syncFolderHint")}
              onclick={addFolder}
            >
              <FolderSymlink size={14} class="shrink-0" aria-hidden="true" />
              {t("shelves.syncFolder")}
            </button>
          </div>
        {/if}
      </div>
    </header>

    {#if documents.length === 0}
      <div class="flex min-h-0 flex-1 flex-col px-4 pb-4 min-[900px]:px-8">
        <div
          class="flex min-h-0 flex-1 flex-col items-center justify-center gap-5 rounded-xl border-2 border-dashed px-8 py-10 text-center {dropActive
            ? 'border-navy-500 bg-navy-100/50 dark:bg-white/10'
            : 'border-paper-line bg-paper-soft/40'}"
          role="region"
          aria-label={t("shelves.emptyRegion")}
        >
          <FolderPlus size={24} class="shrink-0 text-navy-500" aria-hidden="true" />
          <div class="flex flex-col items-center gap-1">
            <p class="text-[0.9375rem] font-medium text-ink">{t("shelves.dropToFill")}</p>
            <p class="max-w-[42ch] text-[13px] text-pretty text-ink-soft">
              {t("shelves.dropTypes")}
            </p>
          </div>
          <div class="flex flex-wrap items-center justify-center gap-2">
            <button type="button" class="btn-primary" onclick={addFiles}>
              <FolderInput size={14} class="shrink-0" aria-hidden="true" />
              {t("shelves.addFiles")}
            </button>
            <button
              type="button"
              class="btn-outline py-2 pr-3 pl-2"
              title={t("shelves.syncFolderHint")}
              onclick={addFolder}
            >
              <FolderSymlink size={14} class="shrink-0" aria-hidden="true" />
              {t("shelves.syncFolder")}
            </button>
          </div>
        </div>
      </div>
    {:else}
      <div class="flex min-h-0 flex-1 flex-col">
        <div
          class="flex shrink-0 flex-wrap items-center justify-between gap-3 border-t border-paper-line px-4 py-2.5 min-[900px]:pr-8 min-[900px]:pl-[29px]"
        >
          <div class="flex min-w-0 flex-wrap items-center gap-2">
            <div class="relative min-w-0">
              <Search
                size={13}
                class="pointer-events-none absolute top-1/2 left-2.5 shrink-0 -translate-y-1/2 text-ink-faint"
                aria-hidden="true"
              />
              <label class="sr-only" for="shelf-filter-name">{t("shelves.filterByName")}</label>
              <input
                id="shelf-filter-name"
                name="filter-name"
                type="text"
                autocomplete="off"
                class="input !w-56 !py-1.5 !pl-8 !text-[12.5px]"
                placeholder={t("shelves.filterByNamePlaceholder")}
                bind:value={searchText}
              />
            </div>
            <label class="sr-only" for="shelf-filter-source">{t("shelves.filterBySource")}</label>
            <span class="select-wrap">
              <select
                id="shelf-filter-source"
                name="filter-source"
                class="input select !w-auto !py-1.5 !text-[12.5px]"
                bind:value={filterSource}
              >
                <option value={null}>{t("shelves.allSources")}</option>
                {#each sourceLabels as label}<option value={label}>{label}</option>{/each}
              </select>
              <svg
                viewBox="0 0 8 5"
                width="8"
                height="5"
                fill="none"
                class="pointer-events-none col-start-2 row-start-1 place-self-center text-ink-faint"
                aria-hidden="true"
              >
                <path d="M.5.5 4 4 7.5.5" stroke="currentcolor" stroke-linecap="round" />
              </svg>
            </span>
            <label class="sr-only" for="shelf-filter-type">{t("shelves.filterByType")}</label>
            <span class="select-wrap">
              <select
                id="shelf-filter-type"
                name="filter-type"
                class="input select !w-auto !py-1.5 !text-[12.5px]"
                bind:value={filterType}
              >
                <option value={null}>{t("shelves.allTypes")}</option>
                {#each fileTypes as type}<option value={type}>{type}</option>{/each}
              </select>
              <svg
                viewBox="0 0 8 5"
                width="8"
                height="5"
                fill="none"
                class="pointer-events-none col-start-2 row-start-1 place-self-center text-ink-faint"
                aria-hidden="true"
              >
                <path d="M.5.5 4 4 7.5.5" stroke="currentcolor" stroke-linecap="round" />
              </svg>
            </span>
            {#if filtersOn}
              <button type="button" class="btn-ghost !py-1 !text-[12px]" onclick={clearFilters}>
                <X size={12} class="shrink-0" aria-hidden="true" />
                {t("shelves.clearFilters")}
              </button>
            {/if}
          </div>
          {#if shelf.linkedFolders.length > 0}
            <SyncedFoldersRow
              folders={shelf.linkedFolders}
              {filterSource}
              onFilter={(name) => (filterSource = name)}
              onUnlink={(linked) => unlinkSource(shelf, linked)}
            />
          {/if}
        </div>

        <div class="min-h-0 flex-1 overflow-y-auto">
          <ShelfDocumentTable
            {visibleDocs}
            onOpen={openFile}
            onRetry={retryFile}
            onClear={clearFilters}
          />
        </div>
      </div>
    {/if}

    <div
      class="flex shrink-0 flex-nowrap items-center justify-between gap-3 border-t border-paper-line px-8 py-2.5"
    >
      <div class="flex min-w-0 flex-1 flex-nowrap items-center gap-1.5">
        {#if documents.length > 0}
          <p
            class="mr-1.5 shrink-0 text-[11.5px] whitespace-nowrap text-ink-faint tabular-nums"
            aria-live="polite"
          >
            {t("shelves.showingOf", {
              visible: visibleDocs.length,
              total: documents.length,
            })}
          </p>
        {/if}
        {#if shelf.stats.pii.total > 0}
          <PiiCategoriesRow
            filesWithPii={shelf.stats.filesWithPii}
            categories={shelf.stats.pii.categories}
            {filterPii}
            onFilter={(value) => (filterPii = value)}
          />
        {/if}
      </div>
      <button
        type="button"
        class="btn-ghost shrink-0 py-2 pr-3 pl-2"
        aria-label={t("shelves.deleteShelf")}
        disabled={deleting}
        onclick={(e) => requestDelete(shelf, e)}
      >
        <Trash2 size={14} class="shrink-0" aria-hidden="true" />
        {t("shelves.delete")}
      </button>
    </div>
  </div>
{/if}

{#if openDoc}
  <DocumentDrawer
    doc={openDoc}
    card={openCard}
    bind:extractedText
    onClose={() => (openDoc = null)}
  />
{/if}
