/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createChatScroll } from "./chat-scroll";

let frames: Map<number, FrameRequestCallback>;
let resize: () => void;
let disconnect: ReturnType<typeof vi.fn>;
let cleanup: (() => void) | undefined;

beforeEach(() => {
  frames = new Map();
  let id = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    frames.set(++id, callback);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  disconnect = vi.fn();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resize = callback;
      }
      observe() {}
      disconnect = disconnect;
    },
  );
});

afterEach(() => {
  cleanup?.();
  document.body.replaceChildren();
  vi.unstubAllGlobals();
});

function paint() {
  const pending = [...frames.values()];
  frames.clear();
  for (const callback of pending) callback(0);
}

function fixture() {
  const viewport = document.createElement("div");
  const content = document.createElement("div");
  viewport.append(content);
  document.body.append(viewport);
  let height = 1000;
  let size = 300;
  let top = 0;
  Object.defineProperties(viewport, {
    scrollHeight: { get: () => height },
    clientHeight: { get: () => size },
    scrollTop: {
      get: () => top,
      set: (value: number) => {
        top = Math.max(0, Math.min(value, height - size));
      },
    },
  });
  const controller = createChatScroll(viewport, content);
  cleanup = controller.destroy;
  paint();
  return {
    viewport,
    content,
    controller,
    grow(by = 20) {
      height += by;
      resize();
    },
    resizeViewport(value: number) {
      size = value;
      resize();
    },
    scroll(value: number) {
      viewport.scrollTop = value;
      viewport.dispatchEvent(new Event("scroll"));
    },
    wheel(deltaY: number) {
      viewport.dispatchEvent(new WheelEvent("wheel", { deltaY }));
    },
    key(key: string, shiftKey = false) {
      viewport.dispatchEvent(new KeyboardEvent("keydown", { key, shiftKey }));
    },
    pointerDown() {
      viewport.dispatchEvent(new MouseEvent("pointerdown", { button: 0 }));
    },
    pointerUp() {
      window.dispatchEvent(new Event("pointerup"));
    },
  };
}

describe("chat scroll following", () => {
  it("follows layout growth in one frame, including changes without a new token", () => {
    const f = fixture();
    expect(f.viewport.scrollTop).toBe(700);
    f.grow();
    f.grow();
    f.grow();
    expect(frames.size).toBe(1);
    paint();
    expect(f.viewport.scrollTop).toBe(760);
  });

  it("yields before an upward wheel event and cancels the pending follow frame", () => {
    const f = fixture();
    f.grow();
    f.wheel(-10);
    f.scroll(690);
    paint();
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(690);
  });

  it("does not reattach on an app-generated scroll event still at the bottom", () => {
    const f = fixture();
    f.wheel(-1);
    f.viewport.dispatchEvent(new Event("scroll"));
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(700);
  });

  it("only reattaches after the user returns to the bottom", () => {
    const f = fixture();
    f.wheel(-30);
    f.scroll(670);
    f.grow();
    paint();
    f.wheel(20);
    f.scroll(690);
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(690);
    f.scroll(740);
    paint();
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(760);
  });

  it("reattaches when a downward wheel step reaches the end before the next layout", () => {
    const f = fixture();
    f.wheel(-20);
    f.scroll(680);
    f.wheel(25);
    f.grow(60);
    paint();
    expect(f.viewport.scrollTop).toBe(760);
  });

  it("End resumes following without a competing native scroll animation", () => {
    const f = fixture();
    f.scroll(400);
    const event = new KeyboardEvent("keydown", { key: "End", cancelable: true });
    f.viewport.dispatchEvent(event);
    f.grow(40);
    paint();
    expect(event.defaultPrevented).toBe(true);
    expect(f.viewport.scrollTop).toBe(740);
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(760);
  });

  it("detects upward native scrollbar movement without a wheel event", () => {
    const f = fixture();
    f.scroll(450);
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(450);
  });

  it("keeps following viewport resizes but preserves a detached reading position", () => {
    const f = fixture();
    f.resizeViewport(200);
    paint();
    expect(f.viewport.scrollTop).toBe(800);
    f.wheel(-100);
    f.scroll(650);
    f.resizeViewport(250);
    paint();
    expect(f.viewport.scrollTop).toBe(650);
  });

  it("does not mistake clamping after content shrinks for an upward gesture", () => {
    const f = fixture();
    f.grow(-100);
    f.scroll(600);
    paint();
    f.grow(40);
    paint();
    expect(f.viewport.scrollTop).toBe(640);
  });

  it.each(["ArrowUp", "PageUp", "Home"])("detaches before native %s scrolling", (key) => {
    const f = fixture();
    f.grow();
    f.key(key);
    paint();
    expect(f.viewport.scrollTop).toBe(700);
    f.scroll(500);
    f.key("End");
    f.scroll(720);
    paint();
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(740);
  });

  it("ignores editing keys and button activation but handles Shift-Space", () => {
    const f = fixture();
    const input = document.createElement("textarea");
    const button = document.createElement("button");
    f.content.append(input, button);
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
    button.dispatchEvent(new KeyboardEvent("keydown", { key: " ", shiftKey: true, bubbles: true }));
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(720);
    f.key(" ", true);
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(720);
  });

  it("lets pointer and touch drags finish without fighting them", () => {
    const f = fixture();
    f.pointerDown();
    f.scroll(500);
    f.grow();
    paint();
    f.pointerUp();
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(500);
    f.pointerDown();
    f.scroll(740);
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(740);
    f.scroll(760);
    f.pointerUp();
    paint();
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(780);
  });

  it("resumes after a click that did not scroll, even if text grew while held", () => {
    const f = fixture();
    f.pointerDown();
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(700);
    f.pointerUp();
    paint();
    expect(f.viewport.scrollTop).toBe(720);
  });

  it("anchors older messages without counting a streamed tail or undoing user scrolling", () => {
    const f = fixture();
    f.scroll(100);
    const anchor = document.createElement("div");
    anchor.dataset.chatMessage = "saved";
    f.content.append(anchor);
    let offset = 120;
    anchor.getBoundingClientRect = () =>
      ({
        top: offset - f.viewport.scrollTop,
        bottom: offset + 100 - f.viewport.scrollTop,
      }) as DOMRect;
    const restore = f.controller.preserveAnchor();
    f.scroll(150);
    offset += 400;
    f.grow(600); // 400 px prepended; 200 px streamed below the anchor.
    restore();
    paint();
    expect(f.viewport.scrollTop).toBe(550);
    f.grow();
    paint();
    expect(f.viewport.scrollTop).toBe(550);
  });

  it("preserves the earlier-message anchor with enlarged text and CSS zoom", () => {
    const f = fixture();
    f.scroll(100);
    Object.defineProperty(f.viewport, "offsetHeight", { get: () => 300 });
    f.viewport.getBoundingClientRect = () => ({ top: 52, height: 390 }) as DOMRect;
    const anchor = document.createElement("div");
    anchor.dataset.chatMessage = "saved";
    f.content.append(anchor);
    let offset = 120;
    anchor.getBoundingClientRect = () =>
      ({
        top: 52 + (offset - f.viewport.scrollTop) * 1.3,
        bottom: 52 + (offset + 100 - f.viewport.scrollTop) * 1.3,
      }) as DOMRect;
    const restore = f.controller.preserveAnchor();
    offset += 400;
    f.grow(500);
    restore();
    paint();
    expect(f.viewport.scrollTop).toBeCloseTo(500);
  });

  it("explicitly follows a new send or conversation after being detached", () => {
    const f = fixture();
    f.scroll(300);
    f.grow();
    paint();
    f.controller.follow();
    paint();
    expect(f.viewport.scrollTop).toBe(720);
  });

  it("cleans up pending work and stale anchor restores when the view closes", () => {
    const f = fixture();
    const restore = f.controller.preserveAnchor();
    f.controller.follow();
    f.grow();
    f.controller.destroy();
    restore();
    resize();
    paint();
    f.wheel(10);
    paint();
    expect(f.viewport.scrollTop).toBe(700);
    expect(disconnect).toHaveBeenCalled();
    expect(frames.size).toBe(0);
  });
});
