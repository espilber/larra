import { describe, expect, it } from "vitest";
import { clipChars, HOUSE_RULES_MAX_CHARS, PROMPT_MAX_CHARS } from "./text-cap";

describe("clipChars", () => {
  it("keeps text under the cap", () => {
    expect(clipChars("hello", 12)).toBe("hello");
  });

  it("cuts at the cap in characters, not bytes", () => {
    const text = "á".repeat(PROMPT_MAX_CHARS + 8);
    const clipped = clipChars(text, PROMPT_MAX_CHARS);
    expect([...clipped].length).toBe(PROMPT_MAX_CHARS);
  });

  it("keeps House rules shorter than a Chat message", () => {
    expect(HOUSE_RULES_MAX_CHARS).toBeLessThan(PROMPT_MAX_CHARS);
  });
});
