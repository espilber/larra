import { describe, expect, it } from "vitest";
import { isMac } from "./platform";
import { parseMenuAction, shortcutAction } from "./shortcuts";

function key(
  key: string,
  mods: { meta?: boolean; ctrl?: boolean; alt?: boolean; shift?: boolean; repeat?: boolean } = {},
): KeyboardEvent {
  return {
    key,
    metaKey: !!mods.meta,
    ctrlKey: !!mods.ctrl,
    altKey: !!mods.alt,
    shiftKey: !!mods.shift,
    repeat: !!mods.repeat,
  } as KeyboardEvent;
}

describe("parseMenuAction", () => {
  it("accepts known actions and ignores others", () => {
    expect(parseMenuAction("new-conversation")).toBe("new-conversation");
    expect(parseMenuAction("view-chat")).toBe("view-chat");
    expect(parseMenuAction("view-settings")).toBe("view-settings");
    expect(parseMenuAction("text-larger")).toBe("text-larger");
    expect(parseMenuAction("text-smaller")).toBe("text-smaller");
    expect(parseMenuAction("nope")).toBeNull();
  });
});

describe("shortcutAction", () => {
  it("maps modifier+N and 1–3", () => {
    const mod = isMac() ? { meta: true } : { ctrl: true };
    expect(shortcutAction(key("n", mod))).toBe("new-conversation");
    expect(shortcutAction(key("1", mod))).toBe("view-chat");
    expect(shortcutAction(key("2", mod))).toBe("view-shelves");
    expect(shortcutAction(key("3", mod))).toBe("view-recipes");
    expect(shortcutAction(key(",", mod))).toBe("view-settings");
    expect(shortcutAction(key("=", mod))).toBe("text-larger");
    expect(shortcutAction(key("+", { ...mod, shift: true }))).toBe("text-larger");
    expect(shortcutAction(key("-", mod))).toBe("text-smaller");
    expect(shortcutAction(key("n"))).toBeNull();
    expect(shortcutAction(key("n", { ...mod, alt: true }))).toBeNull();
  });
});
