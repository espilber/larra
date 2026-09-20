import { t } from "./i18n.svelte";
import type { ChatActivityStep, ChatPrepareStage } from "./api";

/** How long a Thinking line stays up so it can be read. Answer tokens ignore this. */
export const THINKING_STATUS_MIN_MS = 1000;

export const THINKING_STATUS = {
  get fallback() {
    return t("thinking.fallback");
  },
  get warming() {
    return t("thinking.warming");
  },
  get waiting() {
    return t("thinking.waiting");
  },
  get shelf() {
    return t("thinking.shelf");
  },
  get conversation() {
    return t("thinking.conversation");
  },
  get opening() {
    return t("thinking.opening");
  },
  get around() {
    return t("thinking.around");
  },
  get chats() {
    return t("thinking.chats");
  },
  get web() {
    return t("thinking.web");
  },
  get page() {
    return t("thinking.page");
  },
} as const;

function clipFile(file?: string | null): string {
  const name = (file ?? "").trim();
  if (!name) return "";
  return name.length > 56 ? `${name.slice(0, 55)}…` : name;
}

export function openingStatusLine(file?: string | null): string {
  const name = clipFile(file);
  return name ? t("thinking.openingNamed", { file: name }) : THINKING_STATUS.opening;
}

export function aroundStatusLine(file?: string | null): string {
  const name = clipFile(file);
  return name ? t("thinking.aroundNamed", { file: name }) : THINKING_STATUS.around;
}

export function pageStatusLine(file?: string | null): string {
  const name = clipFile(file);
  return name ? t("thinking.pageNamed", { file: name }) : THINKING_STATUS.page;
}

export type ThinkingStatusState = {
  displayed: string;
  shownAt: number;
  queue: string[];
};

export function thinkingStatusLines(input: {
  warming: boolean;
  stage: ChatPrepareStage | null;
  hasShelf: boolean;
  hasHistory: boolean;
  file?: string | null;
}): string[] {
  if (input.warming) return [THINKING_STATUS.warming];
  switch (input.stage) {
    case "waiting":
      return [THINKING_STATUS.waiting];
    case "looking":
      return input.hasShelf ? [THINKING_STATUS.shelf] : [THINKING_STATUS.fallback];
    case "reading": {
      const lines: string[] = [];
      if (input.hasShelf) lines.push(THINKING_STATUS.shelf);
      if (input.hasHistory) lines.push(THINKING_STATUS.conversation);
      return lines.length > 0 ? lines : [THINKING_STATUS.fallback];
    }
    case "opening":
      return [openingStatusLine(input.file)];
    case "around":
      return [aroundStatusLine(input.file)];
    case "chats":
      return [THINKING_STATUS.chats];
    case "web":
      return [THINKING_STATUS.web];
    case "page":
      return [pageStatusLine(input.file)];
    case "thinking":
    case null:
      return [THINKING_STATUS.fallback];
    default: {
      const _exhaustive: never = input.stage;
      return _exhaustive;
    }
  }
}

/** How many look-through steps to keep on a message. */
export const ACTIVITY_MAX_STEPS = 24;

export function isRepeatableActivityStage(stage: ChatPrepareStage): boolean {
  return stage === "opening" || stage === "around";
}

export function appendActivityStep(
  steps: ChatActivityStep[] | undefined,
  stage: ChatPrepareStage,
  file?: string | null,
): ChatActivityStep[] {
  if (stage === "thinking") return steps ?? [];
  const next: ChatActivityStep = file ? { stage, file } : { stage };
  const log = steps ?? [];
  const last = log.at(-1);
  if (
    last &&
    last.stage === stage &&
    (last.file ?? null) === (file ?? null) &&
    !isRepeatableActivityStage(stage)
  ) {
    return log;
  }
  const out = [...log, next];
  return out.length > ACTIVITY_MAX_STEPS ? out.slice(-ACTIVITY_MAX_STEPS) : out;
}

export function visibleActivity(steps?: ChatActivityStep[] | null): ChatActivityStep[] {
  if (!steps?.length) return [];
  const out: ChatActivityStep[] = [];
  for (const step of steps) {
    const stage = parseChatPrepareStage(step.stage);
    if (!stage || stage === "thinking") continue;
    const file = step.file?.trim() || undefined;
    out.push(file ? { stage, file } : { stage });
  }
  return out;
}

function namedFileLinePast(keyNamed: string, keyPlain: string, file?: string | null): string {
  const name = clipFile(file);
  return name ? t(keyNamed, { file: name }) : t(keyPlain);
}

/** Quiet past-tense line for the Thinking log. Live current step stays present tense. */
export function activityStepLabel(step: ChatActivityStep, live = false): string {
  switch (step.stage) {
    case "waiting":
      return live ? t("thinking.waitingLive") : t("thinking.waitingPast");
    case "looking":
      return live ? t("thinking.lookingLive") : t("thinking.lookingPast");
    case "reading":
      return live ? t("thinking.readingLive") : t("thinking.readingPast");
    case "opening":
      return live
        ? namedFileLinePast("thinking.openingNamedLive", "thinking.openingLive", step.file)
        : namedFileLinePast("thinking.openingNamedPast", "thinking.openingPast", step.file);
    case "around":
      return live
        ? namedFileLinePast("thinking.aroundNamedLive", "thinking.aroundLive", step.file)
        : namedFileLinePast("thinking.aroundNamedPast", "thinking.aroundPast", step.file);
    case "chats":
      return live ? t("thinking.chatsLive") : t("thinking.chatsPast");
    case "web":
      return live ? t("thinking.webLive") : t("thinking.webPast");
    case "page":
      return live
        ? namedFileLinePast("thinking.pageNamedLive", "thinking.pageLive", step.file)
        : namedFileLinePast("thinking.pageNamedPast", "thinking.pagePast", step.file);
    case "thinking":
      return live ? t("thinking.thinkingLive") : t("thinking.thinkingPast");
    default: {
      const _exhaustive: never = step.stage;
      return _exhaustive;
    }
  }
}

export function parseChatPrepareStage(value: string | undefined): ChatPrepareStage | undefined {
  switch (value) {
    case "waiting":
    case "looking":
    case "reading":
    case "opening":
    case "around":
    case "chats":
    case "web":
    case "page":
    case "thinking":
      return value;
    default:
      return undefined;
  }
}

/** First specific line shows at once. Later lines wait `minMs` so they can be read. */
export function enqueueThinkingLines(
  state: ThinkingStatusState,
  lines: string[],
  now: number,
): ThinkingStatusState {
  const incoming = uniqueConsecutive(lines.filter((line) => line.length > 0));
  if (incoming.length === 0) return state;

  let displayed = state.displayed;
  let shownAt = state.shownAt;
  const queue = [...state.queue];

  if (!displayed) {
    displayed = incoming[0] ?? THINKING_STATUS.fallback;
    shownAt = now;
    for (const line of incoming.slice(1)) {
      appendUnique(queue, displayed, line);
    }
    return { displayed, shownAt, queue };
  }

  const replaceFallback =
    displayed === THINKING_STATUS.fallback && incoming[0] !== THINKING_STATUS.fallback;

  for (const line of incoming) {
    appendUnique(queue, displayed, line);
  }

  if (replaceFallback && queue.length > 0) {
    displayed = queue.shift() ?? displayed;
    shownAt = now;
  }

  return { displayed, shownAt, queue };
}

export function advanceThinkingStatus(
  state: ThinkingStatusState,
  now: number,
  minMs: number,
): { displayed: string; shownAt: number; queue: string[]; waitMs: number | null } {
  if (state.queue.length === 0) {
    return { ...state, queue: [], waitMs: null };
  }
  const elapsed = now - state.shownAt;
  if (elapsed < minMs) {
    return { ...state, queue: [...state.queue], waitMs: minMs - elapsed };
  }
  const queue = [...state.queue];
  const displayed = queue.shift() ?? state.displayed;
  return {
    displayed,
    shownAt: now,
    queue,
    waitMs: queue.length > 0 ? minMs : null,
  };
}

function uniqueConsecutive(lines: string[]): string[] {
  const out: string[] = [];
  for (const line of lines) {
    if (out.at(-1) !== line) out.push(line);
  }
  return out;
}

function appendUnique(queue: string[], displayed: string, line: string) {
  const last = queue.at(-1) ?? displayed;
  if (line !== last) queue.push(line);
}
