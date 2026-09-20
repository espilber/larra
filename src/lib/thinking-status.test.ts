import { describe, expect, it } from "vitest";
import type { ChatPrepareStage } from "./api";
import {
  ACTIVITY_MAX_STEPS,
  THINKING_STATUS,
  THINKING_STATUS_MIN_MS,
  activityStepLabel,
  advanceThinkingStatus,
  appendActivityStep,
  enqueueThinkingLines,
  parseChatPrepareStage,
  thinkingStatusLines,
  visibleActivity,
  type ThinkingStatusState,
} from "./thinking-status";

function empty(_now = 0): ThinkingStatusState {
  return { displayed: "", shownAt: 0, queue: [] };
}

function showing(line: string, at = 0, queue: string[] = []): ThinkingStatusState {
  return { displayed: line, shownAt: at, queue };
}

describe("thinkingStatusLines", () => {
  it("keeps Warming up over every other stage", () => {
    expect(
      thinkingStatusLines({
        warming: true,
        stage: "reading",
        hasShelf: true,
        hasHistory: true,
      }),
    ).toEqual([THINKING_STATUS.warming]);
  });

  it("names waiting, looking, and thinking in plain language", () => {
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "waiting",
        hasShelf: false,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.waiting]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "looking",
        hasShelf: true,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.shelf]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "thinking",
        hasShelf: true,
        hasHistory: true,
      }),
    ).toEqual([THINKING_STATUS.fallback]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: null,
        hasShelf: false,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.fallback]);
  });

  it("names the file being opened", () => {
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "opening",
        hasShelf: true,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.opening]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "opening",
        hasShelf: true,
        hasHistory: false,
        file: "Staff handbook.md",
      }),
    ).toEqual(["Opening Staff handbook.md…"]);
  });

  it("names reading more of a file and earlier conversations", () => {
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "around",
        hasShelf: true,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.around]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "around",
        hasShelf: true,
        hasHistory: false,
        file: "Staff handbook.md",
      }),
    ).toEqual(["Reading more of Staff handbook.md…"]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "chats",
        hasShelf: false,
        hasHistory: true,
      }),
    ).toEqual([THINKING_STATUS.chats]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "web",
        hasShelf: false,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.web]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "page",
        hasShelf: false,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.page]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "page",
        hasShelf: false,
        hasHistory: false,
        file: "en.wikipedia.org",
      }),
    ).toEqual(["Opening en.wikipedia.org…"]);
  });

  it("only mentions a Shelf or earlier messages when they are in play", () => {
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "reading",
        hasShelf: true,
        hasHistory: true,
      }),
    ).toEqual([THINKING_STATUS.shelf, THINKING_STATUS.conversation]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "reading",
        hasShelf: true,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.shelf]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "reading",
        hasShelf: false,
        hasHistory: true,
      }),
    ).toEqual([THINKING_STATUS.conversation]);
    expect(
      thinkingStatusLines({
        warming: false,
        stage: "reading",
        hasShelf: false,
        hasHistory: false,
      }),
    ).toEqual([THINKING_STATUS.fallback]);
  });

  it("covers every prepare stage", () => {
    const stages: ChatPrepareStage[] = [
      "waiting",
      "looking",
      "reading",
      "opening",
      "around",
      "chats",
      "web",
      "page",
      "thinking",
    ];
    for (const stage of stages) {
      expect(
        thinkingStatusLines({
          warming: false,
          stage,
          hasShelf: true,
          hasHistory: true,
        }).length,
      ).toBeGreaterThan(0);
    }
  });
});

describe("appendActivityStep", () => {
  it("skips thinking and dedupes a repeated look", () => {
    let log = appendActivityStep([], "looking");
    log = appendActivityStep(log, "looking");
    log = appendActivityStep(log, "thinking");
    log = appendActivityStep(log, "opening", "notes.md");
    log = appendActivityStep(log, "opening", "notes.md");
    expect(log).toEqual([
      { stage: "looking" },
      { stage: "opening", file: "notes.md" },
      { stage: "opening", file: "notes.md" },
    ]);
  });

  it("keeps the latest steps when the log runs long", () => {
    let log = appendActivityStep([], "looking");
    for (let i = 0; i < ACTIVITY_MAX_STEPS + 2; i++) {
      log = appendActivityStep(log, "opening", `part-${i}.md`);
    }
    expect(log).toHaveLength(ACTIVITY_MAX_STEPS);
    expect(log[0]?.file).toBe("part-2.md");
  });
});

describe("visibleActivity", () => {
  it("drops thinking and unknown stages", () => {
    expect(
      visibleActivity([
        { stage: "looking" },
        { stage: "thinking" },
        { stage: "opening", file: "  notes.md  " },
      ]),
    ).toEqual([{ stage: "looking" }, { stage: "opening", file: "notes.md" }]);
  });
});

describe("activityStepLabel", () => {
  it("names finished steps in the past tense", () => {
    expect(activityStepLabel({ stage: "looking" })).toBe("Looked through your files");
    expect(activityStepLabel({ stage: "opening", file: "Staff handbook.md" })).toBe(
      "Opened Staff handbook.md",
    );
    expect(activityStepLabel({ stage: "around", file: "Staff handbook.md" })).toBe(
      "Read more of Staff handbook.md",
    );
    expect(activityStepLabel({ stage: "web" })).toBe("Looked on the web");
  });

  it("keeps the current live step in the present tense", () => {
    expect(activityStepLabel({ stage: "opening", file: "notes.md" }, true)).toBe(
      "Opening notes.md",
    );
    expect(activityStepLabel({ stage: "chats" }, true)).toBe("Reading earlier conversations");
  });
});

describe("parseChatPrepareStage", () => {
  it("accepts known stages and drops anything else", () => {
    expect(parseChatPrepareStage("waiting")).toBe("waiting");
    expect(parseChatPrepareStage("looking")).toBe("looking");
    expect(parseChatPrepareStage("reading")).toBe("reading");
    expect(parseChatPrepareStage("thinking")).toBe("thinking");
    expect(parseChatPrepareStage("opening")).toBe("opening");
    expect(parseChatPrepareStage("around")).toBe("around");
    expect(parseChatPrepareStage("chats")).toBe("chats");
    expect(parseChatPrepareStage("web")).toBe("web");
    expect(parseChatPrepareStage("page")).toBe("page");
    expect(parseChatPrepareStage("queued")).toBeUndefined();
    expect(parseChatPrepareStage(undefined)).toBeUndefined();
  });
});

describe("enqueueThinkingLines", () => {
  it("shows the first line immediately", () => {
    const next = enqueueThinkingLines(empty(), [THINKING_STATUS.shelf], 40);
    expect(next.displayed).toBe(THINKING_STATUS.shelf);
    expect(next.shownAt).toBe(40);
    expect(next.queue).toEqual([]);
  });

  it("replaces the generic Thinking line at once with a more specific one", () => {
    const next = enqueueThinkingLines(
      showing(THINKING_STATUS.fallback, 10),
      [THINKING_STATUS.shelf, THINKING_STATUS.conversation],
      50,
    );
    expect(next.displayed).toBe(THINKING_STATUS.shelf);
    expect(next.shownAt).toBe(50);
    expect(next.queue).toEqual([THINKING_STATUS.conversation]);
  });

  it("replaces Thinking at once when a file is opening", () => {
    const next = enqueueThinkingLines(
      showing(THINKING_STATUS.fallback, 10),
      ["Opening notes.md…"],
      40,
    );
    expect(next.displayed).toBe("Opening notes.md…");
    expect(next.queue).toEqual([]);
  });

  it("queues later lines so they can sit for the minimum read time", () => {
    const next = enqueueThinkingLines(
      showing(THINKING_STATUS.shelf, 20),
      [THINKING_STATUS.shelf, THINKING_STATUS.conversation, THINKING_STATUS.fallback],
      80,
    );
    expect(next.displayed).toBe(THINKING_STATUS.shelf);
    expect(next.shownAt).toBe(20);
    expect(next.queue).toEqual([THINKING_STATUS.conversation, THINKING_STATUS.fallback]);
  });

  it("does not queue a duplicate of what is already showing", () => {
    const next = enqueueThinkingLines(
      showing(THINKING_STATUS.fallback, 0),
      [THINKING_STATUS.fallback],
      10,
    );
    expect(next.displayed).toBe(THINKING_STATUS.fallback);
    expect(next.queue).toEqual([]);
  });
});

describe("advanceThinkingStatus", () => {
  it("holds the current line until the minimum read time", () => {
    const held = advanceThinkingStatus(
      showing(THINKING_STATUS.shelf, 0, [THINKING_STATUS.conversation]),
      THINKING_STATUS_MIN_MS - 1,
      THINKING_STATUS_MIN_MS,
    );
    expect(held.displayed).toBe(THINKING_STATUS.shelf);
    expect(held.queue).toEqual([THINKING_STATUS.conversation]);
    expect(held.waitMs).toBe(1);
  });

  it("advances when the minimum read time has passed", () => {
    const next = advanceThinkingStatus(
      showing(THINKING_STATUS.shelf, 0, [THINKING_STATUS.conversation, THINKING_STATUS.fallback]),
      THINKING_STATUS_MIN_MS,
      THINKING_STATUS_MIN_MS,
    );
    expect(next.displayed).toBe(THINKING_STATUS.conversation);
    expect(next.queue).toEqual([THINKING_STATUS.fallback]);
    expect(next.waitMs).toBe(THINKING_STATUS_MIN_MS);
    expect(next.shownAt).toBe(THINKING_STATUS_MIN_MS);
  });

  it("stops scheduling when the queue is empty", () => {
    const idle = advanceThinkingStatus(showing(THINKING_STATUS.fallback, 0), 5000, 1000);
    expect(idle.displayed).toBe(THINKING_STATUS.fallback);
    expect(idle.waitMs).toBeNull();
  });
});
