<script lang="ts">
  import { api, type SourcePassage, type StoredMessage } from "$lib/api";
  import { importIntoChat } from "$lib/chat-import";
  import { createChatScroll, type ChatScroll } from "$lib/chat-scroll";
  import { listenFileDrop } from "$lib/files";
  import {
    app,
    chatState,
    newConversation,
    ensureActiveThread,
    fillDraft,
    notifyInvokeError,
    rememberPreferredShelf,
    openThread,
    loadOlderMessages,
    refreshThreads,
  } from "$lib/stores.svelte";
  import { clipChars, PROMPT_MAX_CHARS } from "$lib/text-cap";
  import { groupSourceChips, type SourceChip } from "$lib/source-chips";
  import {
    isOutboundPlaceholder,
    outboundPlaceholderId,
    pendingForThread,
    threadIsBusy,
  } from "$lib/chat-reducer";
  import Markdown from "$lib/components/Markdown.svelte";
  import CopyActions from "$lib/components/CopyActions.svelte";
  import SourcePanel from "$lib/components/SourcePanel.svelte";
  import ChatThreadList from "$lib/components/ChatThreadList.svelte";
  import ChatThreadHeader from "$lib/components/ChatThreadHeader.svelte";
  import ChatComposer from "$lib/components/ChatComposer.svelte";
  import ChatEmptyState from "$lib/components/ChatEmptyState.svelte";
  import ThinkingStatus from "$lib/components/ThinkingStatus.svelte";
  import ThinkingPanel from "$lib/components/ThinkingPanel.svelte";
  import ConversationFace from "$lib/components/ConversationFace.svelte";
  import { confirmDanger } from "$lib/native-dialog";
  import { t } from "$lib/i18n.svelte";
  import { shot } from "$lib/shot-control.svelte";
  import { tick } from "svelte";
  import { MediaQuery } from "svelte/reactivity";
  import { PanelLeft, Plus } from "@lucide/svelte";

  const narrow = new MediaQuery("(max-width: 899px)");
  let conversationsDialog = $state<HTMLDialogElement | null>(null);

  $effect(() => {
    if (!narrow.current || app.threads.length === 0) conversationsDialog?.close();
  });

  async function chooseThread(id: string) {
    try {
      await openThread(id);
      conversationsDialog?.close();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  function startConversation() {
    conversationsDialog?.close();
    newConversation();
  }

  let composerEl = $state<HTMLTextAreaElement | null>(null);
  let scrollEl = $state<HTMLDivElement | null>(null);
  let scrollContentEl = $state<HTMLDivElement | null>(null);
  let scrollController = $state<ChatScroll | null>(null);
  let openSource = $state<SourcePassage | null>(null);
  let openThinking = $state<Record<string, boolean>>({});
  let dropActive = $state(false);
  let openedStartSource = $state(false);
  let openedStartThinking = $state(false);

  const activeThread = $derived(app.threads.find((t) => t.id === chatState.activeThreadId) ?? null);
  const hasModel = $derived(!!app.settings?.activeModel);
  const activePending = $derived(pendingForThread(chatState.pending, chatState.activeThreadId));
  const warming = $derived(activePending?.phase === "queued" && app.engine.state !== "ready");
  const pendingStage = $derived(
    activePending?.stage ?? (activePending?.phase === "streaming" ? "thinking" : null),
  );
  const pendingHasShelf = $derived(
    !!(chatState.selectedShelfId || chatState.uploadShelf || activeThread?.uploadShelfId),
  );
  const pendingHasHistory = $derived(chatState.messages.length > 1);
  const generating = $derived(
    threadIsBusy(chatState.pending, chatState.outbound, chatState.activeThreadId),
  );
  const empty = $derived(chatState.messages.length === 0 && !generating);

  $effect(() => {
    if (!activeThread) return;
    const shelfId = activeThread.shelfId ?? null;
    const uploadId = activeThread.uploadShelfId ?? null;
    chatState.selectedShelfId = shelfId && shelfId !== uploadId ? shelfId : null;
  });

  $effect(() => {
    return listenFileDrop({
      onOver: (active) => (dropActive = active),
      onDrop: (paths) => {
        void importIntoChat(paths);
      },
    });
  });

  $effect(() => {
    if (openedStartSource) return;
    if (!shot.source && import.meta.env.VITE_START_SOURCE !== "first") return;
    const cited = [...chatState.messages].reverse().find((message) => message.sources.length > 0);
    if (!cited?.sources[0]) return;
    openedStartSource = true;
    openSource = cited.sources[0] ?? null;
  });

  $effect(() => {
    if (openedStartThinking) return;
    if (!shot.thinking && import.meta.env.VITE_START_THINKING !== "first") return;
    const withThink = [...chatState.messages]
      .reverse()
      .find((message) => message.thinking || (message.activity?.length ?? 0) > 0);
    if (!withThink) return;
    openedStartThinking = true;
    openThinking = { ...openThinking, [withThink.id]: true };
  });

  function citationPage(source: SourcePassage): string | null {
    if (!source.pageStart) return null;
    if (source.pageEnd && source.pageEnd !== source.pageStart) {
      return t("chat.pageRange", { start: source.pageStart, end: source.pageEnd });
    }
    return t("chat.page", { start: source.pageStart });
  }

  function chipAriaLabel(chip: SourceChip): string {
    const parts = [chip.sids.join(" · "), chip.source.title];
    if (chip.showSection && chip.source.section) parts.push(chip.source.section);
    const page = citationPage(chip.source);
    if (page) parts.push(page);
    return parts.filter(Boolean).join(" ");
  }

  async function chooseShelf(shelfId: string | null) {
    rememberPreferredShelf(shelfId);
    chatState.selectedShelfId = shelfId;
    if (!chatState.activeThreadId) return;
    try {
      await api.threadSetShelf(chatState.activeThreadId, shelfId);
      await refreshThreads();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  $effect(() => {
    if (chatState.draftFocus > 0 && composerEl) {
      void chatState.draftFocus;
      tick().then(() => {
        autoresize();
        composerEl?.focus();
        const marker = chatState.draft.indexOf("«");
        if (marker >= 0) {
          const end = chatState.draft.indexOf("»", marker);
          composerEl?.setSelectionRange(marker, end >= 0 ? end + 1 : marker);
        } else {
          const end = chatState.draft.length;
          composerEl?.setSelectionRange(end, end);
        }
      });
    }
  });

  function markOutbound(threadId: string) {
    chatState.outbound = { ...chatState.outbound, [threadId]: true };
  }

  function clearOutbound(threadId: string) {
    if (!chatState.outbound[threadId]) return;
    const next = { ...chatState.outbound };
    delete next[threadId];
    chatState.outbound = next;
  }

  function setPlaceholderPending(threadId: string) {
    chatState.pending = {
      ...chatState.pending,
      [threadId]: {
        messageId: outboundPlaceholderId(threadId),
        threadId,
        text: "",
        thinking: "",
        phase: "queued",
      },
    };
  }

  function dropPending(threadId: string) {
    if (!chatState.pending[threadId]) return;
    const next = { ...chatState.pending };
    delete next[threadId];
    chatState.pending = next;
  }

  function clearCancelWhenQueued(threadId: string) {
    if (!chatState.cancelWhenQueued[threadId]) return;
    const next = { ...chatState.cancelWhenQueued };
    delete next[threadId];
    chatState.cancelWhenQueued = next;
  }

  async function send() {
    const text = chatState.draft.trim();
    if (!text || generating || !hasModel) return;
    const navigation = chatState.navigation;
    const shelfId = chatState.selectedShelfId;
    const originalDraft = chatState.draft;
    const optimisticId = `local-${crypto.randomUUID()}`;
    let outboundKey = chatState.activeThreadId ?? "new";
    markOutbound(outboundKey);
    if (chatState.activeThreadId) {
      setPlaceholderPending(chatState.activeThreadId);
    }
    try {
      while (
        (chatState.imports[outboundKey] ?? 0) > 0 ||
        (outboundKey === "new" && (chatState.imports.new ?? 0) > 0)
      ) {
        if (chatState.cancelWhenQueued[outboundKey]) {
          clearOutbound(outboundKey);
          dropPending(outboundKey);
          clearCancelWhenQueued(outboundKey);
          return;
        }
        await new Promise((resolve) => setTimeout(resolve, 20));
      }
      let threadId = outboundKey === "new" ? null : outboundKey;
      if (!threadId) {
        if (chatState.navigation !== navigation) {
          clearOutbound(outboundKey);
          clearCancelWhenQueued(outboundKey);
          return;
        }
        threadId = await ensureActiveThread();
        if (chatState.outbound["new"]) {
          clearOutbound("new");
          markOutbound(threadId);
        }
        if (chatState.cancelWhenQueued["new"]) {
          const next = { ...chatState.cancelWhenQueued };
          delete next["new"];
          next[threadId] = true;
          chatState.cancelWhenQueued = next;
        }
        outboundKey = threadId;
        setPlaceholderPending(threadId);
      }
      const optimistic: StoredMessage = {
        id: optimisticId,
        role: "user",
        text,
        ts: new Date().toISOString(),
        shelfId,
        sources: [],
        status: "done",
      };
      if (chatState.navigation === navigation) {
        chatState.messages = [...chatState.messages, optimistic];
        chatState.draft = "";
      }
      if (chatState.navigation === navigation) scrollController?.follow();
      await tick();
      autoresize();
      chatState.sentDrafts[threadId] = originalDraft;
      await api.chatSend(threadId, text, shelfId);
    } catch (error) {
      delete chatState.sentDrafts[outboundKey];
      clearOutbound(outboundKey);
      dropPending(outboundKey);
      clearCancelWhenQueued(outboundKey);
      clearCancelWhenQueued("new");
      if (chatState.navigation === navigation) {
        chatState.messages = chatState.messages.filter((m) => m.id !== optimisticId);
        if (!chatState.draft) fillDraft(originalDraft);
      } else chatState.drafts[outboundKey] = originalDraft;
      notifyInvokeError(error);
    }
  }

  function retry(message: StoredMessage) {
    const index = chatState.messages.findIndex((m) => m.id === message.id);
    const question = chatState.messages
      .slice(0, index)
      .reverse()
      .find((m) => m.role === "user");
    if (question) fillDraft(question.text);
    composerEl?.focus();
  }

  function stop() {
    const threadId = chatState.activeThreadId ?? "new";
    const inFlight = pendingForThread(chatState.pending, chatState.activeThreadId);
    if (inFlight && !isOutboundPlaceholder(inFlight.messageId)) {
      api.chatCancel(inFlight.messageId).catch(notifyInvokeError);
      return;
    }
    chatState.cancelWhenQueued = { ...chatState.cancelWhenQueued, [threadId]: true };
  }

  function autoresize() {
    if (!composerEl) return;
    composerEl.style.height = "auto";
    composerEl.style.height = `${Math.min(composerEl.scrollHeight, 180)}px`;
  }

  async function readMore() {
    const navigation = chatState.navigation;
    const restore = scrollController?.preserveAnchor();
    try {
      await loadOlderMessages();
    } catch (error) {
      notifyInvokeError(error);
      return;
    }
    await tick();
    if (navigation === chatState.navigation) restore?.();
  }

  $effect(() => {
    if (!scrollEl || !scrollContentEl) return;
    const controller = createChatScroll(scrollEl, scrollContentEl);
    scrollController = controller;
    return () => {
      controller.destroy();
      scrollController = null;
    };
  });

  $effect(() => {
    void chatState.navigation;
    scrollController?.follow();
  });

  async function removeThread(threadId: string) {
    const thread = app.threads.find((t) => t.id === threadId);
    const count =
      threadId === chatState.activeThreadId
        ? Math.max(thread?.messageCount ?? 0, chatState.messages.length)
        : (thread?.messageCount ?? 0);
    if (count >= 5) {
      const ok = await confirmDanger(t("chat.deleteConversation"), t("chat.delete"));
      if (!ok) return;
    }
    try {
      await api.threadDelete(threadId);
      if (chatState.activeThreadId === threadId) {
        chatState.activeThreadId = null;
        chatState.messages = [];
        chatState.uploadShelf = null;
        chatState.hasOlder = false;
      }
      const inFlight = chatState.pending[threadId];
      if (inFlight && !isOutboundPlaceholder(inFlight.messageId)) {
        api.chatCancel(inFlight.messageId).catch(notifyInvokeError);
      } else if (chatState.outbound[threadId]) {
        chatState.cancelWhenQueued = { ...chatState.cancelWhenQueued, [threadId]: true };
      }
      dropPending(threadId);
      clearOutbound(threadId);
      await refreshThreads();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function renameThread(threadId: string, title: string) {
    try {
      await api.threadRename(threadId, title);
      await refreshThreads();
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function exportThread(threadId: string) {
    try {
      await api.threadExport(threadId);
    } catch (error) {
      notifyInvokeError(error);
    }
  }
</script>

<div class="relative flex h-full min-h-0">
  {#if dropActive}
    <div
      class="pointer-events-none absolute inset-3 z-30 flex items-center justify-center rounded-2xl border-2 border-dashed border-navy-500 bg-navy-100/50 dark:bg-white/10"
    >
      <p class="rounded-xl bg-navy-900 px-4 py-2 text-[13.5px] font-medium text-white shadow-pop">
        {t("chat.dropHint")}
      </p>
    </div>
  {/if}

  {#snippet threadList(drawer = false)}
    <ChatThreadList
      threads={app.threads}
      activeThreadId={chatState.activeThreadId}
      shelves={app.shelves}
      onOpen={chooseThread}
      onNew={startConversation}
      onRemove={removeThread}
      onRename={renameThread}
      onExport={exportThread}
      onClose={drawer ? () => conversationsDialog?.close() : undefined}
    />
  {/snippet}
  {#if narrow.current}
    <dialog
      bind:this={conversationsDialog}
      aria-label={t("chat.conversations")}
      class="fixed inset-y-4 left-[80px] m-0 h-[calc(100%-2rem)] max-h-none max-w-[calc(100%-96px)] overflow-visible border-0 bg-transparent p-0 backdrop:bg-navy-950/30 dark:backdrop:bg-black/50"
    >
      {@render threadList(true)}
    </dialog>
  {:else}
    {@render threadList()}
  {/if}

  <section class="flex min-w-0 flex-1 flex-col">
    {#if narrow.current}
      <div class="flex shrink-0 items-center gap-2 border-b border-paper-line px-3 py-2">
        {#if app.threads.length > 0}
          <button
            type="button"
            class="btn-ghost !px-2"
            aria-haspopup="dialog"
            onclick={() => conversationsDialog?.showModal()}
          >
            <PanelLeft size={17} aria-hidden="true" />{t("chat.conversations")}
          </button>
        {/if}
        <button
          type="button"
          class="btn-ghost ml-auto !p-2"
          onclick={startConversation}
          aria-label={t("chat.newConversation")}
          title={t("chat.newConversation")}
        >
          <Plus size={17} aria-hidden="true" />
        </button>
      </div>
    {/if}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex (the scroll region supports native keyboard scrolling) -->
    <div
      bind:this={scrollEl}
      tabindex="0"
      role="region"
      aria-label={t("nav.chat")}
      class="chat-scroll min-h-0 flex-1 overflow-y-auto [mask-image:linear-gradient(to_bottom,black_0%,black_calc(100%-25px),transparent_100%)] focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-navy-500"
    >
      {#if empty}
        <ChatEmptyState avatarId={activeThread?.avatarId} />
      {:else}
        <div
          bind:this={scrollContentEl}
          class="mx-auto flex max-w-[760px] flex-col gap-4 px-3 pt-4 pb-6 min-[900px]:px-6"
        >
          {#if activeThread}
            {@const thread = activeThread}
            <ChatThreadHeader
              {thread}
              shelves={app.shelves}
              messageCount={Math.max(thread.messageCount, chatState.messages.length)}
              onDownload={() => void exportThread(thread.id)}
            />
          {/if}
          {#if chatState.hasOlder}
            <div class="flex justify-center">
              <button
                type="button"
                class="btn-outline !py-1.5 !text-[12.5px]"
                onclick={() => void readMore()}
                disabled={chatState.loadingOlder}
                aria-label={t("chat.readEarlier")}
              >
                {t("chat.readMore")}
              </button>
            </div>
          {/if}
          {#each chatState.messages as message (message.id)}
            {#if message.role === "user"}
              <div data-chat-message={message.id} class="flex justify-end">
                <div
                  class="max-w-[90%] cursor-text rounded-2xl rounded-br-md bg-navy-900 px-4 py-2.5 text-[13.8px] leading-relaxed break-words whitespace-pre-wrap text-white select-text"
                >
                  {message.text}
                </div>
              </div>
            {:else}
              <div data-chat-message={message.id} class="group flex items-start gap-2.5">
                {#if activeThread}
                  <ConversationFace avatarId={activeThread.avatarId} />
                {/if}
                <div class="flex min-w-0 flex-1 flex-col gap-1.5">
                  <div
                    class="max-w-[92%] rounded-2xl rounded-tl-md border border-paper-line bg-surface px-4 py-3 shadow-card dark:shadow-none"
                  >
                    <ThinkingPanel
                      id={message.id}
                      open={!!openThinking[message.id]}
                      onToggle={() =>
                        (openThinking = {
                          ...openThinking,
                          [message.id]: !openThinking[message.id],
                        })}
                      thinking={message.thinking}
                      activity={message.activity}
                    />
                    <Markdown
                      text={message.text}
                      sources={message.sources}
                      onCite={(s) => (openSource = s)}
                    />
                    {#if message.sources.length > 0}
                      <div class="mt-2.5 flex flex-wrap gap-1.5 border-t border-paper-line pt-2.5">
                        {#each groupSourceChips(message.sources) as chip (chip.key)}
                          {@const page = citationPage(chip.source)}
                          <button
                            type="button"
                            class="chip border border-navy-200 bg-navy-50 text-navy-700 hover:border-navy-500 hover:bg-navy-200/60 dark:border-white/10 dark:bg-white/8 dark:text-navy-100 dark:hover:border-navy-400 dark:hover:bg-white/12"
                            aria-label={chipAriaLabel(chip)}
                            onclick={() => (openSource = chip.source)}
                          >
                            <span class="font-bold">{chip.sids.join(" · ")}</span>
                            <span class="max-w-[220px] truncate font-normal"
                              >{chip.source.title}</span
                            >
                            {#if chip.showSection && chip.source.section}
                              <span class="max-w-[140px] truncate text-navy-400 dark:text-navy-300"
                                >{chip.source.section}</span
                              >
                            {/if}
                            {#if page}
                              <span class="text-navy-400 dark:text-navy-300">{page}</span>
                            {/if}
                          </button>
                        {/each}
                      </div>
                    {/if}
                    {#if ["stopped", "error", "interrupted"].includes(message.status)}
                      <p class="mt-2 text-[12px] text-ink-soft">
                        {t(message.status === "stopped" ? "chat.stopped" : "chat.interrupted")}
                      </p>
                      <div class="mt-2 flex flex-wrap gap-2">
                        <button
                          class="btn-ghost"
                          disabled={generating}
                          onclick={() => retry(message)}>{t("shelves.tryAgain")}</button
                        >
                        {#if message.text}<button
                            class="btn-ghost"
                            disabled={generating}
                            onclick={() => fillDraft(t("chat.continuePrompt"))}
                            >{t("chat.continue")}</button
                          >{/if}
                      </div>
                    {/if}
                  </div>
                  <CopyActions text={message.text} subtle />
                </div>
              </div>
            {/if}
          {/each}

          {#if activePending}
            <div class="flex items-start gap-2.5" aria-label={t("chat.answerInProgress")}>
              {#if activeThread}
                <ConversationFace avatarId={activeThread.avatarId} />
              {/if}
              <div class="flex min-w-0 flex-1 flex-col gap-1.5">
                <div
                  class="max-w-[92%] rounded-2xl rounded-tl-md border border-paper-line bg-surface px-4 py-3 shadow-card dark:shadow-none"
                >
                  {#if !activePending.text}
                    <ThinkingPanel
                      id="pending"
                      live
                      open={!!openThinking.pending}
                      onToggle={() =>
                        (openThinking = { ...openThinking, pending: !openThinking.pending })}
                      thinking={activePending.thinking}
                      activity={activePending.activity}
                    >
                      {#snippet lead()}
                        <span
                          class="inline-block size-2 shrink-0 animate-pulse rounded-full bg-navy-500 motion-reduce:animate-none"
                        ></span>
                        <span class="min-w-0 grow">
                          {#key activePending.threadId}
                            <ThinkingStatus
                              {warming}
                              stage={pendingStage}
                              hasShelf={pendingHasShelf}
                              hasHistory={pendingHasHistory}
                              file={activePending.file}
                            />
                          {/key}
                        </span>
                      {/snippet}
                    </ThinkingPanel>
                  {:else}
                    <ThinkingPanel
                      id="pending"
                      live
                      open={!!openThinking.pending}
                      onToggle={() =>
                        (openThinking = { ...openThinking, pending: !openThinking.pending })}
                      thinking={activePending.thinking}
                      activity={activePending.activity}
                    />
                    <Markdown text={activePending.text} streaming />
                  {/if}
                </div>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>

    <div class="sr-only" role="status" aria-live="polite" aria-atomic="true">
      {chatState.webApprovals[chatState.activeThreadId ?? ""]
        ? t("chat.reviewOnline")
        : activePending
          ? activePending.text
            ? t("chat.answerInProgress")
            : t("chat.preparingAnswer")
          : chatState.messages.at(-1)?.role === "assistant"
            ? t(
                chatState.messages.at(-1)?.status === "done"
                  ? "chat.answerComplete"
                  : "chat.interrupted",
              )
            : ""}
    </div>
    {#if chatState.webApprovals[chatState.activeThreadId ?? ""]}
      {@const request = chatState.webApprovals[chatState.activeThreadId ?? ""]!}
      <div
        class="mx-3 rounded-xl border border-paper-line bg-paper-soft p-3 min-[900px]:mx-6"
        role="region"
        aria-label={t("chat.reviewOnline")}
      >
        <p class="font-medium text-ink">{t("chat.reviewOnline")}</p>
        <p class="mt-1 text-sm text-ink-soft">
          {t(request.action === "search_web" ? "chat.queryLeaves" : "chat.urlLeaves")}
        </p>
        <pre
          class="my-2 max-h-40 overflow-auto text-sm break-all whitespace-pre-wrap select-text">{request.value}</pre>
        <div class="flex flex-wrap gap-2">
          <button
            class="btn-primary"
            onclick={() => api.chatApproveWeb(request.id, true).catch(notifyInvokeError)}
            >{t("chat.allowOnce")}</button
          >
          <button
            class="btn-ghost"
            onclick={() => api.chatApproveWeb(request.id, false).catch(notifyInvokeError)}
            >{t("chat.keepLocal")}</button
          >
        </div>
      </div>
    {/if}
    <ChatComposer
      bind:composerEl
      {hasModel}
      {generating}
      onSend={send}
      onStop={stop}
      onChooseShelf={chooseShelf}
      onAutoResize={autoresize}
    />
  </section>
</div>

{#if openSource}
  <SourcePanel source={openSource} onClose={() => (openSource = null)} />
{/if}
