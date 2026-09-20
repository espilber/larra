/** @vitest-environment jsdom */
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DocumentMeta, EngineStatus, ThreadPage } from "./api";

const mock = vi.hoisted(() => ({
  handlers: new Map<string, (event: unknown) => void>(),
  failListener: false,
  removed: 0,
  api: {
    shelvesList: vi.fn(),
    threadsList: vi.fn(),
    settingsGet: vi.fn(),
    engineStatus: vi.fn(),
    updateInfo: vi.fn(),
    threadMessages: vi.fn(),
    shelfGet: vi.fn(),
    shelfDocuments: vi.fn(),
    pickFiles: vi.fn(),
    threadEnsureUploadShelf: vi.fn(),
    shelfImportPaths: vi.fn(),
  },
}));
vi.mock("./api", async (importOriginal) => {
  const original = await importOriginal<typeof import("./api")>();
  const events = Object.fromEntries(
    ["engine", "download", "ingest", "webApproval", "shelfStats", "shelves", "chat", "update"].map(
      (name) => [
        name,
        (handler: (event: unknown) => void) => {
          if (mock.failListener && name === "ingest")
            return Promise.reject(new Error("listen failed"));
          mock.handlers.set(name, handler);
          return Promise.resolve(() => {
            mock.removed++;
            if (mock.handlers.get(name) === handler) mock.handlers.delete(name);
          });
        },
      ],
    ),
  );
  return { ...original, api: { ...original.api, ...mock.api }, events };
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => {
    resolve = r;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.resetModules();
  vi.clearAllMocks();
  mock.handlers.clear();
  mock.failListener = false;
  mock.removed = 0;
  mock.api.shelvesList.mockResolvedValue([]);
  mock.api.threadsList.mockResolvedValue([]);
  mock.api.settingsGet.mockResolvedValue({
    onboardingDone: true,
    activeModel: null,
    houseRules: "",
    textSize: "default",
    uiLocale: "en",
    resolvedLocale: "en",
  });
  mock.api.engineStatus.mockResolvedValue({ state: "no-model" });
  mock.api.updateInfo.mockResolvedValue(null);
  mock.api.threadMessages.mockResolvedValue({ messages: [], hasOlder: false });
});

describe("asynchronous app state", () => {
  it("subscribes before snapshots and replays a newer engine event", async () => {
    const state = await import("./stores.svelte");
    const snapshot = deferred<EngineStatus>();
    mock.api.engineStatus.mockReturnValue(snapshot.promise);
    const starting = state.bootstrap();
    await vi.waitFor(() => expect(mock.api.engineStatus).toHaveBeenCalled());
    mock.handlers.get("engine")!({ state: "ready" });
    snapshot.resolve({ state: "no-model" });
    await starting;
    expect(state.app.engine.state).toBe("ready");
    expect(state.app.ready).toBe(true);
  });
  it("cleans failed subscriptions and permits a complete retry", async () => {
    const state = await import("./stores.svelte");
    mock.failListener = true;
    await expect(state.bootstrap()).rejects.toThrow();
    expect(state.app.bootstrapError).toBe(true);
    expect(mock.removed).toBe(7);
    expect(mock.api.engineStatus).not.toHaveBeenCalled();
    mock.failListener = false;
    await state.bootstrap();
    expect(state.app.ready).toBe(true);
    expect(mock.handlers.size).toBe(8);
  });
  it("rejects a stale A-to-B-to-A conversation load", async () => {
    const state = await import("./stores.svelte");
    const old = deferred<ThreadPage>();
    mock.api.threadMessages.mockReturnValueOnce(old.promise);
    const first = state.openThread("a");
    await state.openThread("b");
    await state.openThread("a");
    old.resolve({
      messages: [
        { id: "stale", role: "user", text: "Wrong view", ts: "", sources: [], status: "done" },
      ],
      hasOlder: true,
    });
    await first;
    expect(state.chatState.messages).toEqual([]);
    expect(state.chatState.hasOlder).toBe(false);
  });
  it("merges document changes received during a snapshot load", async () => {
    const state = await import("./stores.svelte");
    await state.bootstrap();
    const snapshot = deferred<DocumentMeta[]>();
    mock.api.shelfDocuments.mockReturnValue(snapshot.promise);
    const loading = state.loadShelfDocuments("s");
    const doc = { id: "fresh", shelfId: "s", fileName: "new.md", status: "ready" } as DocumentMeta;
    mock.handlers.get("ingest")!({ shelfId: "s", documentId: "old", status: "removed" });
    mock.handlers.get("ingest")!({
      shelfId: "s",
      documentId: "fresh",
      status: "ready",
      document: doc,
    });
    snapshot.resolve([{ ...doc, id: "old" }]);
    await loading;
    expect(state.app.documents.s).toEqual([doc]);
    await state.loadShelfDocuments("s");
    expect(mock.api.shelfDocuments).toHaveBeenCalledTimes(1);
  });
  it("keeps a file-picker import on its original conversation", async () => {
    const state = await import("./stores.svelte");
    await state.openThread("a");
    const picker = deferred<string[] | null>();
    mock.api.pickFiles.mockReturnValue(picker.promise);
    mock.api.threadEnsureUploadShelf.mockResolvedValue({ id: "uploads-a" });
    mock.api.shelfImportPaths.mockResolvedValue({ queued: 1, names: ["notes.md"], atLimit: false });
    const { importIntoChat } = await import("./chat-import");
    const importing = importIntoChat();
    await state.openThread("b");
    state.chatState.draft = "B draft";
    picker.resolve(["/synthetic/notes.md"]);
    await importing;
    expect(mock.api.threadEnsureUploadShelf).toHaveBeenCalledWith("a");
    expect(state.chatState.activeThreadId).toBe("b");
    expect(state.chatState.draft).toBe("B draft");
    expect(state.chatState.uploadShelf).toBeNull();
    expect(state.chatState.drafts.a).toBe("notes.md");
  });
});

it("reconciles optimistic user messages and restores an unacknowledged failed draft", async () => {
  const state = await import("./stores.svelte");
  await state.bootstrap();
  state.chatState.activeThreadId = "t";
  const user = {
    id: "local-1",
    role: "user" as const,
    text: "Question",
    ts: "",
    status: "done" as const,
    sources: [],
  };
  state.chatState.messages = [user, { ...user, id: "saved-user" }];
  mock.handlers.get("chat")!({
    kind: "queued",
    threadId: "t",
    messageId: "assistant",
    userMessageId: "saved-user",
  });
  expect(state.chatState.messages.map((m) => m.id)).toEqual(["saved-user"]);
  state.chatState.messages = [user];
  state.chatState.sentDrafts.t = "Question";
  state.chatState.draft = "";
  mock.handlers.get("chat")!({
    kind: "error",
    threadId: "t",
    messageId: "",
    error: "Save failed.",
  });
  expect(state.chatState.draft).toBe("Question");
  expect(state.chatState.messages).toEqual([]);
});
