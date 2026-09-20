import type { ChatActivityStep, ChatEvent, ChatPrepareStage, StoredMessage } from "./api";
import { clipChars, THINKING_MAX_CHARS } from "./text-cap";
import { appendActivityStep, parseChatPrepareStage } from "./thinking-status";

export type ChatPending = {
  messageId: string;
  threadId: string;
  text: string;
  thinking: string;
  phase: "queued" | "streaming";
  stage?: ChatPrepareStage;
  file?: string;
  activity?: ChatActivityStep[];
};

export type ChatPendingMap = Record<string, ChatPending>;

export type ChatReduceResult = {
  pending: ChatPendingMap;
  append?: StoredMessage;
  error?: string;
  refreshThreads?: boolean;
};

/** In-flight assistant text for this thread, or null if none. */
export function pendingForThread(
  pending: ChatPendingMap,
  threadId: string | null,
): ChatPending | null {
  if (!threadId) return null;
  return pending[threadId] ?? null;
}

const OUTBOUND_PREFIX = "outbound-";

export function outboundPlaceholderId(threadId: string): string {
  return `${OUTBOUND_PREFIX}${threadId}`;
}

export function isOutboundPlaceholder(messageId: string): boolean {
  return messageId.startsWith(OUTBOUND_PREFIX);
}

/** True when this conversation already has a send in flight or waiting. */
export function threadIsBusy(
  pending: ChatPendingMap,
  outbound: Record<string, boolean>,
  threadId: string | null,
): boolean {
  if (outbound[threadId ?? "new"]) return true;
  return pendingForThread(pending, threadId) !== null;
}

export function reduceChatEvent(pending: ChatPendingMap, event: ChatEvent): ChatReduceResult {
  const current = pending[event.threadId] ?? null;
  const inner = reduceOne(current, event);
  const next = { ...pending };
  if (inner.pending) {
    next[event.threadId] = inner.pending;
  } else {
    delete next[event.threadId];
  }
  return {
    pending: next,
    append: inner.append,
    error: inner.error,
    refreshThreads: inner.refreshThreads,
  };
}

type OneResult = {
  pending: ChatPending | null;
  append?: StoredMessage;
  error?: string;
  refreshThreads?: boolean;
};

function reduceOne(pending: ChatPending | null, event: ChatEvent): OneResult {
  switch (event.kind) {
    case "queued":
      return {
        pending: {
          messageId: event.messageId,
          threadId: event.threadId,
          text: "",
          thinking: "",
          phase: "queued",
        },
      };
    case "status": {
      const stage = parseChatPrepareStage(event.stage);
      if (!stage) return { pending };
      if (pending && !isThisTurn(pending, event) && !isOutboundPlaceholder(pending.messageId)) {
        return { pending };
      }
      const base =
        pending && (isThisTurn(pending, event) || isOutboundPlaceholder(pending.messageId))
          ? pending
          : {
              messageId: event.messageId,
              threadId: event.threadId,
              text: "",
              thinking: "",
              phase: "queued" as const,
            };
      return {
        pending: {
          ...base,
          messageId: event.messageId || base.messageId,
          stage,
          file: event.file,
          activity: appendActivityStep(base.activity, stage, event.file),
        },
      };
    }
    case "clear": {
      if (!pending || (!isThisTurn(pending, event) && !isOutboundPlaceholder(pending.messageId))) {
        return { pending };
      }
      return { pending: { ...pending, text: "" } };
    }
    case "started": {
      const next = adopt(pending, event);
      if (next && next.messageId === event.messageId) {
        return { pending: { ...next, phase: "streaming" } };
      }
      return { pending: next };
    }
    case "delta": {
      const next = adopt(pending, event);
      if (next && next.messageId === event.messageId && event.text) {
        return { pending: { ...next, text: next.text + event.text, phase: "streaming" } };
      }
      return { pending: next };
    }
    case "thinking": {
      const next = adopt(pending, event);
      if (next && next.messageId === event.messageId && event.text) {
        return {
          pending: {
            ...next,
            thinking: clipChars(next.thinking + event.text, THINKING_MAX_CHARS),
            phase: "streaming",
          },
        };
      }
      return { pending: next };
    }
    case "promote": {
      if (!pending || pending.messageId !== event.messageId) {
        return { pending };
      }
      return {
        pending: {
          ...pending,
          thinking: clipChars(pending.thinking + pending.text, THINKING_MAX_CHARS),
          text: "",
        },
      };
    }
    case "done": {
      const message = event.message;
      const append = message;
      return {
        pending: isThisTurn(pending, event) ? null : pending,
        append,
        refreshThreads: true,
      };
    }
    case "error": {
      return {
        pending:
          !event.messageId ||
          (isThisTurn(pending, event) && pending && isOutboundPlaceholder(pending.messageId))
            ? null
            : pending,
        error: event.error,
      };
    }
    default: {
      const _exhaustive: never = event.kind;
      return _exhaustive;
    }
  }
}

function isThisTurn(pending: ChatPending | null, event: ChatEvent): boolean {
  if (!pending) return true;
  if (isOutboundPlaceholder(pending.messageId)) return true;
  if (!event.messageId) return true;
  return pending.messageId === event.messageId;
}

function adopt(pending: ChatPending | null, event: ChatEvent): ChatPending | null {
  if (pending && !isOutboundPlaceholder(pending.messageId)) {
    return pending;
  }
  if (event.kind === "delta" || event.kind === "thinking" || event.kind === "started") {
    const next: ChatPending = {
      messageId: event.messageId,
      threadId: event.threadId,
      text: "",
      thinking: "",
      phase: "streaming",
    };
    if (pending?.stage) next.stage = pending.stage;
    if (pending?.file) next.file = pending.file;
    if (pending?.activity?.length) next.activity = pending.activity;
    return next;
  }
  return null;
}
