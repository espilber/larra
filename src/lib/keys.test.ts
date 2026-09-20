/** @vitest-environment jsdom */

import { describe, expect, it } from "vitest";
import { isTextInput, suppressWebviewBeep } from "./keys";
import { isTextualTarget } from "./native-menu";

function key(
  key: string,
  target: EventTarget | null,
  mods: { meta?: boolean; ctrl?: boolean } = {},
): KeyboardEvent {
  const event = new KeyboardEvent("keydown", {
    key,
    metaKey: !!mods.meta,
    ctrlKey: !!mods.ctrl,
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "target", { value: target });
  return event;
}

describe("isTextInput", () => {
  it("treats text fields as inputs and buttons as not", () => {
    const input = document.createElement("input");
    const area = document.createElement("textarea");
    const button = document.createElement("button");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    expect(isTextInput(input)).toBe(true);
    expect(isTextInput(area)).toBe(true);
    expect(isTextInput(button)).toBe(false);
    expect(isTextInput(checkbox)).toBe(false);
  });
});

describe("suppressWebviewBeep", () => {
  it("prevents character keys outside fields", () => {
    const event = key("n", document.body);
    suppressWebviewBeep(event);
    expect(event.defaultPrevented).toBe(true);
  });

  it("leaves Space, shortcuts, and typing alone", () => {
    const input = document.createElement("input");
    const space = key(" ", document.body);
    const shortcut = key("n", document.body, { meta: true });
    const typing = key("n", input);
    suppressWebviewBeep(space);
    suppressWebviewBeep(shortcut);
    suppressWebviewBeep(typing);
    expect(space.defaultPrevented).toBe(false);
    expect(shortcut.defaultPrevented).toBe(false);
    expect(typing.defaultPrevented).toBe(false);
  });
});

describe("isTextualTarget", () => {
  it("matches selectable chat text", () => {
    const bubble = document.createElement("div");
    bubble.className = "select-text";
    expect(isTextualTarget(bubble)).toBe(true);
    expect(isTextualTarget(document.body)).toBe(false);
  });
});
