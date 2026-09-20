/** @vitest-environment jsdom */

import { describe, expect, it } from "vitest";
import {
  applyTextSize,
  parseTextSize,
  persistTextSize,
  restoreTextSize,
  stepTextSize,
  TEXT_SIZE_KEY,
  textSizeFromIndex,
  textSizeIndex,
} from "./text-size";

describe("parseTextSize", () => {
  it("keeps known sizes and falls back to default", () => {
    expect(parseTextSize("default")).toBe("default");
    expect(parseTextSize("large")).toBe("large");
    expect(parseTextSize("larger")).toBe("larger");
    expect(parseTextSize("huge")).toBe("default");
    expect(parseTextSize(undefined)).toBe("default");
  });
});

describe("stepTextSize", () => {
  it("moves one step and stops at the ends", () => {
    expect(textSizeIndex("default")).toBe(0);
    expect(textSizeFromIndex(9)).toBe("larger");
    expect(stepTextSize("default", 1)).toBe("large");
    expect(stepTextSize("large", 1)).toBe("larger");
    expect(stepTextSize("larger", 1)).toBe("larger");
    expect(stepTextSize("larger", -1)).toBe("large");
    expect(stepTextSize("default", -1)).toBe("default");
  });
});

describe("applyTextSize", () => {
  it("sets and clears the root data attribute", () => {
    const root = document.createElement("html");
    applyTextSize("large", root);
    expect(root.dataset.textSize).toBe("large");
    applyTextSize("default", root);
    expect(root.dataset.textSize).toBeUndefined();
  });
});

describe("persistTextSize", () => {
  it("roundtrips through storage", () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
    };
    persistTextSize("larger", storage);
    expect(store.get(TEXT_SIZE_KEY)).toBe("larger");
    const root = document.createElement("html");
    expect(restoreTextSize(storage, root)).toBe("larger");
    expect(root.dataset.textSize).toBe("larger");
  });
});
