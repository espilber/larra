import { describe, expect, it } from "vitest";
import { virtualWindow } from "./virtual-window";

describe("large document tables", () => {
  it("keeps mounted rows bounded at the start, middle and end", () => {
    for (const top of [0, 12000, 1e9]) {
      const range = virtualWindow(1000, top, 480, 48);
      expect(range.end - range.start).toBeLessThanOrEqual(22);
      expect(range.before + (range.end - range.start) * 48 + range.after).toBe(48000);
      expect(range.start).toBeGreaterThanOrEqual(0);
    }
  });
  it("clamps after filtering and handles larger rows", () => {
    expect(virtualWindow(3, 10000, 480, 60)).toEqual({ start: 0, end: 3, before: 0, after: 0 });
    expect(virtualWindow(0, 100, 480, 60).end).toBe(0);
  });
});
