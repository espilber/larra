import { api, type ImportResult } from "./api";
import { pinFileNames } from "./placeholders";
import { shelfCapMessage, skippedLongMessage } from "./shelf-cap";
import {
  chatState,
  ensureActiveThread,
  fillDraft,
  notify,
  notifyInvokeError,
  refreshThreads,
} from "./stores.svelte";
import { t } from "./i18n.svelte";

export type ChatImportResult = "done" | "cancelled";

function chatImportNotice(result: ImportResult): string {
  const longMsg = skippedLongMessage(result.skippedLong ?? 0);
  const message = result.atLimit
    ? shelfCapMessage(result.queued)
    : result.queued === 0
      ? t("imports.unsupported")
      : t(result.queued === 1 ? "imports.addedOne" : "imports.addedMany", { count: result.queued });
  return longMsg ? `${message} ${longMsg}` : message;
}

/** Capture the destination before showing a native picker; navigation cannot retarget an import. */
export async function importIntoChat(paths?: string[]): Promise<ChatImportResult> {
  const navigation = chatState.navigation;
  let threadId = chatState.activeThreadId;
  const key = threadId ?? "new";
  chatState.imports[key] = (chatState.imports[key] ?? 0) + 1;
  try {
    const picked = paths ?? (await api.pickFiles());
    if (!picked?.length) return "cancelled";
    if (!threadId) {
      if (chatState.navigation !== navigation) return "cancelled";
      threadId = await ensureActiveThread();
    }
    const shelf = await api.threadEnsureUploadShelf(threadId);
    if (chatState.activeThreadId === threadId && chatState.navigation === navigation)
      chatState.uploadShelf = shelf;
    await refreshThreads();
    const result = await api.shelfImportPaths(shelf.id, picked);
    if (result.queued > 0) {
      if (chatState.activeThreadId === threadId && chatState.navigation === navigation)
        fillDraft(pinFileNames(chatState.draft, result.names));
      else
        chatState.drafts[threadId] = pinFileNames(chatState.drafts[threadId] ?? "", result.names);
    }
    notify(chatImportNotice(result));
    return "done";
  } catch (error) {
    notifyInvokeError(error);
    return "done";
  } finally {
    chatState.imports[key] = Math.max(0, (chatState.imports[key] ?? 1) - 1);
  }
}
