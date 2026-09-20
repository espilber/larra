import { describe, expect, it } from "vitest";
import { THINKING_MAX_CHARS } from "./text-cap";
import {
  outboundPlaceholderId,
  pendingForThread,
  reduceChatEvent,
  threadIsBusy,
  type ChatPending,
  type ChatPendingMap,
} from "./chat-reducer";
import type { ChatEvent, StoredMessage } from "./api";

function base(kind: ChatEvent["kind"], extra: Partial<ChatEvent> = {}): ChatEvent {
  return {
    threadId: "t1",
    messageId: "m1",
    kind,
    ...extra,
  };
}

function pending(extra: Partial<ChatPending> = {}): ChatPending {
  return {
    messageId: "m1",
    threadId: "t1",
    text: "",
    thinking: "",
    phase: "queued",
    ...extra,
  };
}

function mapOf(item: ChatPending | null): ChatPendingMap {
  return item ? { [item.threadId]: item } : {};
}

function at(out: { pending: ChatPendingMap }, threadId = "t1"): ChatPending | undefined {
  return out.pending[threadId];
}

const doneMessage: StoredMessage = {
  id: "m1",
  role: "assistant",
  text: "Hello world",
  ts: "now",
  sources: [],
  status: "done",
};

describe("reduceChatEvent", () => {
  it("opens a queued pending message", () => {
    const out = reduceChatEvent({}, base("queued"));
    expect(at(out)?.phase).toBe("queued");
    expect(at(out)?.messageId).toBe("m1");
    expect(at(out)?.text).toBe("");
  });

  it("records prepare stages without starting the stream", () => {
    let next = reduceChatEvent({}, base("queued")).pending;
    next = reduceChatEvent(next, base("status", { stage: "reading" })).pending;
    expect(at({ pending: next })?.stage).toBe("reading");
    expect(at({ pending: next })?.phase).toBe("queued");

    next = reduceChatEvent(next, base("status", { stage: "thinking" })).pending;
    expect(at({ pending: next })?.stage).toBe("thinking");
    expect(at({ pending: next })?.phase).toBe("queued");
  });

  it("records an opening file name and can clear streamed text", () => {
    let next = reduceChatEvent({}, base("queued")).pending;
    next = reduceChatEvent(next, base("status", { stage: "opening", file: "notes.md" })).pending;
    expect(at({ pending: next })?.stage).toBe("opening");
    expect(at({ pending: next })?.file).toBe("notes.md");
    expect(at({ pending: next })?.activity).toEqual([{ stage: "opening", file: "notes.md" }]);

    next = reduceChatEvent(next, base("delta", { text: "{" })).pending;
    expect(at({ pending: next })?.text).toBe("{");
    next = reduceChatEvent(next, base("clear")).pending;
    expect(at({ pending: next })?.text).toBe("");
    expect(at({ pending: next })?.stage).toBe("opening");
  });

  it("ignores a status event with no stage", () => {
    const current = mapOf(pending());
    expect(reduceChatEvent(current, base("status")).pending).toEqual(current);
  });

  it("accumulates look-through steps and skips a thinking beat", () => {
    let next = reduceChatEvent({}, base("queued")).pending;
    next = reduceChatEvent(next, base("status", { stage: "looking" })).pending;
    next = reduceChatEvent(next, base("status", { stage: "looking" })).pending;
    next = reduceChatEvent(next, base("status", { stage: "opening", file: "notes.md" })).pending;
    next = reduceChatEvent(next, base("status", { stage: "thinking" })).pending;
    expect(at({ pending: next })?.activity).toEqual([
      { stage: "looking" },
      { stage: "opening", file: "notes.md" },
    ]);
    expect(at({ pending: next })?.stage).toBe("thinking");
  });

  it("keeps the look-through log when the stream starts from a placeholder", () => {
    const placeholder = pending({
      messageId: outboundPlaceholderId("t1"),
      phase: "queued",
      stage: "opening",
      file: "notes.md",
      activity: [{ stage: "opening", file: "notes.md" }],
    });
    const out = reduceChatEvent(mapOf(placeholder), base("started"));
    expect(at(out)?.activity).toEqual([{ stage: "opening", file: "notes.md" }]);
    expect(at(out)?.stage).toBe("opening");
  });

  it("keeps the prepare stage after the stream starts", () => {
    const out = reduceChatEvent(mapOf(pending({ stage: "thinking" })), base("started"));
    expect(at(out)?.stage).toBe("thinking");
    expect(at(out)?.phase).toBe("streaming");
  });

  it("adopts a status event over an outbound placeholder", () => {
    const placeholder = pending({
      messageId: outboundPlaceholderId("t1"),
      phase: "queued",
    });
    const out = reduceChatEvent(mapOf(placeholder), base("status", { stage: "waiting" }));
    expect(at(out)?.messageId).toBe("m1");
    expect(at(out)?.stage).toBe("waiting");
    expect(at(out)?.phase).toBe("queued");
  });

  it("keeps another conversation's pending when status arrives for this one", () => {
    const other = pending({ threadId: "t2", messageId: "m2", phase: "streaming", text: "keep" });
    const out = reduceChatEvent(mapOf(other), base("status", { stage: "reading" }));
    expect(at(out, "t2")?.text).toBe("keep");
    expect(at(out)?.stage).toBe("reading");
  });

  it("started ends the warming-up queued phase", () => {
    const out = reduceChatEvent(mapOf(pending()), base("started"));
    expect(at(out)?.phase).toBe("streaming");
    expect(at(out)?.messageId).toBe("m1");
    expect(out.error).toBeUndefined();
  });

  it("appends deltas then clears on done", () => {
    let next = reduceChatEvent({}, base("queued")).pending;
    next = reduceChatEvent(next, base("started")).pending;
    next = reduceChatEvent(next, base("delta", { text: "Hello" })).pending;
    next = reduceChatEvent(next, base("delta", { text: " world" })).pending;
    expect(next.t1?.text).toBe("Hello world");
    expect(next.t1?.phase).toBe("streaming");
    const done = reduceChatEvent(next, base("done", { message: doneMessage }));
    expect(done.pending).toEqual({});
    expect(done.append?.text).toBe("Hello world");
    expect(done.refreshThreads).toBe(true);
  });

  it("appends thinking deltas separately from the answer", () => {
    let next = mapOf(pending({ phase: "streaming" }));
    next = reduceChatEvent(next, base("thinking", { text: "hmm " })).pending;
    next = reduceChatEvent(next, base("thinking", { text: "ok" })).pending;
    expect(next.t1?.thinking).toBe("hmm ok");
    expect(next.t1?.text).toBe("");
  });

  it("caps stored thinking", () => {
    const chunk = "á".repeat(100);
    let next = mapOf(pending({ phase: "streaming" }));
    for (let i = 0; i < THINKING_MAX_CHARS / 100 + 4; i++) {
      next = reduceChatEvent(next, base("thinking", { text: chunk })).pending;
    }
    expect([...next.t1!.thinking].length).toBe(THINKING_MAX_CHARS);
  });

  it("promotes answer text into thinking", () => {
    const out = reduceChatEvent(
      mapOf(pending({ text: "scratch", thinking: "pre ", phase: "streaming" })),
      base("promote"),
    );
    expect(at(out)?.thinking).toBe("pre scratch");
    expect(at(out)?.text).toBe("");
  });

  it("retains partial text while an engine error is persisted", () => {
    const out = reduceChatEvent(
      mapOf(pending({ phase: "streaming", text: "partial" })),
      base("error", { error: "Larra couldn't finish that answer. Try again." }),
    );
    expect(out.pending.t1?.text).toBe("partial");
    expect(out.append).toBeUndefined();
    expect(out.error).toBe("Larra couldn't finish that answer. Try again.");
  });

  it("persists a cancelled or failed answer so it can be retried", () => {
    const stopped = reduceChatEvent(
      mapOf(pending({ phase: "streaming" })),
      base("done", { message: { ...doneMessage, status: "error" } }),
    );
    expect(stopped.pending).toEqual({});
    expect(stopped.append?.status).toBe("error");
    expect(stopped.refreshThreads).toBe(true);

    const cancelled = reduceChatEvent(
      mapOf(pending({ phase: "streaming" })),
      base("done", { message: { ...doneMessage, status: "stopped", text: "half" } }),
    );
    expect(cancelled.append?.status).toBe("stopped");
    expect(cancelled.append?.text).toBe("half");
  });

  it("adopts a late started or delta when pending is missing", () => {
    const started = reduceChatEvent({}, base("started"));
    expect(at(started)).toEqual({
      messageId: "m1",
      threadId: "t1",
      text: "",
      thinking: "",
      phase: "streaming",
    });

    const delta = reduceChatEvent({}, base("delta", { text: "Hi" }));
    expect(at(delta)?.text).toBe("Hi");
    expect(at(delta)?.phase).toBe("streaming");
  });

  it("ignores deltas for a different in-flight message", () => {
    const current = pending({ phase: "streaming", text: "keep" });
    const out = reduceChatEvent(mapOf(current), base("delta", { messageId: "m2", text: "other" }));
    expect(at(out)?.messageId).toBe("m1");
    expect(at(out)?.text).toBe("keep");
  });

  it("does not clear pending when a different message finishes or errors", () => {
    const current = pending({ phase: "streaming", text: "keep", messageId: "m2", threadId: "t2" });
    const done = reduceChatEvent(mapOf(current), base("done", { message: doneMessage }));
    expect(at(done, "t2")?.messageId).toBe("m2");
    expect(done.append?.text).toBe("Hello world");

    const failed = reduceChatEvent(mapOf(current), base("error", { error: "nope" }));
    expect(at(failed, "t2")?.messageId).toBe("m2");
    expect(failed.error).toBe("nope");
  });

  it("keeps one conversation's stream when another is queued", () => {
    const first = pending({ phase: "streaming", text: "keep", threadId: "t1", messageId: "m1" });
    const out = reduceChatEvent(mapOf(first), base("queued", { threadId: "t2", messageId: "m2" }));
    expect(at(out, "t1")?.text).toBe("keep");
    expect(at(out, "t2")?.phase).toBe("queued");
    expect(at(out, "t2")?.messageId).toBe("m2");

    const done = reduceChatEvent(out.pending, base("done", { message: doneMessage }));
    expect(at(done, "t1")).toBeUndefined();
    expect(at(done, "t2")?.messageId).toBe("m2");
  });

  it("hides in-flight text that belongs to another thread", () => {
    const current = mapOf(pending({ threadId: "t1", phase: "streaming", text: "secret" }));
    expect(pendingForThread(current, "t1")?.text).toBe("secret");
    expect(pendingForThread(current, "t2")).toBeNull();
    expect(pendingForThread(current, null)).toBeNull();
    expect(pendingForThread({}, "t1")).toBeNull();
  });

  it("promote and empty delta do nothing without pending text", () => {
    expect(reduceChatEvent({}, base("promote")).pending).toEqual({});
    const same = mapOf(pending({ phase: "streaming", text: "x" }));
    expect(at(reduceChatEvent(same, base("delta")))?.text).toBe("x");
  });

  it("preserves a started stream's stage from a placeholder", () => {
    const placeholder = pending({
      messageId: outboundPlaceholderId("t1"),
      phase: "queued",
      stage: "reading",
    });
    const started = reduceChatEvent(mapOf(placeholder), base("started"));
    expect(at(started)?.stage).toBe("reading");
    expect(at(started)?.phase).toBe("streaming");
  });

  it("adopts a started stream over an outbound placeholder", () => {
    const placeholder = pending({
      messageId: outboundPlaceholderId("t1"),
      phase: "queued",
    });
    const started = reduceChatEvent(mapOf(placeholder), base("started"));
    expect(at(started)?.messageId).toBe("m1");
    expect(at(started)?.phase).toBe("streaming");

    const delta = reduceChatEvent(mapOf(placeholder), base("delta", { text: "Hi" }));
    expect(at(delta)?.text).toBe("Hi");
    expect(at(delta)?.messageId).toBe("m1");
  });

  it("clears an outbound placeholder when the real message finishes or errors", () => {
    const placeholder = mapOf(pending({ messageId: outboundPlaceholderId("t1"), phase: "queued" }));
    const done = reduceChatEvent(placeholder, base("done", { message: doneMessage }));
    expect(done.pending).toEqual({});
    expect(done.append?.text).toBe("Hello world");

    const failed = reduceChatEvent(placeholder, base("error", { error: "nope" }));
    expect(failed.pending).toEqual({});
    expect(failed.error).toBe("nope");
  });

  it("clears in-flight state when an error has no message id", () => {
    const current = mapOf(pending({ phase: "queued" }));
    const out = reduceChatEvent(current, base("error", { messageId: "", error: "nope" }));
    expect(out.pending).toEqual({});
    expect(out.error).toBe("nope");
  });
});

describe("threadIsBusy", () => {
  it("treats an outbound lock as busy before queued arrives", () => {
    expect(threadIsBusy({}, { t1: true }, "t1")).toBe(true);
    expect(threadIsBusy({}, { new: true }, null)).toBe(true);
    expect(threadIsBusy({}, { t1: true }, "t2")).toBe(false);
    expect(threadIsBusy({}, {}, "t1")).toBe(false);
  });

  it("treats in-flight pending as busy", () => {
    const current = mapOf(pending({ threadId: "t2" }));
    expect(threadIsBusy(current, {}, "t2")).toBe(true);
    expect(threadIsBusy(current, {}, "t1")).toBe(false);
  });
});

it("keeps partial text until the interrupted answer is persisted", () => {
  let state = reduceChatEvent({}, { kind: "queued", threadId: "t", messageId: "a" }).pending;
  state = reduceChatEvent(state, {
    kind: "delta",
    threadId: "t",
    messageId: "a",
    text: "Partial answer",
  }).pending;
  const error = reduceChatEvent(state, {
    kind: "error",
    threadId: "t",
    messageId: "a",
    error: "Interrupted.",
  });
  expect(error.pending.t?.text).toBe("Partial answer");
  const message: StoredMessage = {
    id: "a",
    role: "assistant",
    text: "Partial answer",
    status: "interrupted",
    ts: "",
    sources: [],
  };
  const done = reduceChatEvent(error.pending, {
    kind: "done",
    threadId: "t",
    messageId: "a",
    message,
  });
  expect(done.append).toEqual(message);
  expect(done.pending.t).toBeUndefined();
});
