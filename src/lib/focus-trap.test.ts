/** @vitest-environment jsdom */

import { afterEach, describe, expect, it } from "vitest";
import { focusTrap } from "./focus-trap";

function layout(el: HTMLElement) {
  Object.defineProperty(el, "getClientRects", {
    value: () => [{ width: 8, height: 8 }],
  });
}

function tab(root: HTMLElement, shiftKey = false) {
  root.dispatchEvent(
    new KeyboardEvent("keydown", { key: "Tab", shiftKey, bubbles: true, cancelable: true }),
  );
}

function button(label: string, extra?: (el: HTMLButtonElement) => void): HTMLButtonElement {
  const el = document.createElement("button");
  el.textContent = label;
  layout(el);
  extra?.(el);
  return el;
}

describe("focusTrap", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("moves focus to the first control and cycles Tab both ways", () => {
    const root = document.createElement("div");
    const first = button("First");
    const last = button("Last");
    root.append(first, last);
    document.body.append(root);

    const trap = focusTrap(root);
    expect(document.activeElement).toBe(first);

    last.focus();
    tab(root);
    expect(document.activeElement).toBe(first);

    tab(root, true);
    expect(document.activeElement).toBe(last);

    trap.destroy();
  });

  it("does not wrap Tab when focus is in the middle", () => {
    const root = document.createElement("div");
    const first = button("First");
    const mid = button("Mid");
    const last = button("Last");
    root.append(first, mid, last);
    document.body.append(root);

    focusTrap(root);
    mid.focus();
    tab(root);
    expect(document.activeElement).toBe(mid);
    tab(root, true);
    expect(document.activeElement).toBe(mid);
  });

  it("skips disabled, hidden, and aria-hidden controls", () => {
    const root = document.createElement("div");
    const hiddenWrap = document.createElement("div");
    hiddenWrap.setAttribute("aria-hidden", "true");
    hiddenWrap.append(button("Ghost"));
    const disabled = button("Off", (el) => el.setAttribute("disabled", ""));
    const hidden = button("Hidden", (el) => el.setAttribute("hidden", ""));
    const first = button("First");
    const last = button("Last");
    root.append(hiddenWrap, disabled, hidden, first, last);
    document.body.append(root);

    focusTrap(root);
    expect(document.activeElement).toBe(first);
    last.focus();
    tab(root);
    expect(document.activeElement).toBe(first);
  });

  it("does nothing when there is no focusable control", () => {
    const outside = button("Outside");
    document.body.append(outside);
    outside.focus();
    const root = document.createElement("div");
    document.body.append(root);

    const trap = focusTrap(root);
    expect(document.activeElement).toBe(outside);
    tab(root);
    expect(document.activeElement).toBe(outside);
    trap.destroy();
  });

  it("restores the previous focus and stops trapping on destroy", () => {
    const outside = button("Outside");
    document.body.append(outside);
    outside.focus();

    const root = document.createElement("div");
    const first = button("First");
    const last = button("Last");
    root.append(first, last);
    document.body.append(root);

    const trap = focusTrap(root);
    expect(document.activeElement).toBe(first);
    trap.destroy();
    expect(document.activeElement).toBe(outside);

    last.focus();
    tab(root);
    expect(document.activeElement).toBe(last);
  });
});
