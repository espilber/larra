import { describe, expect, it } from "vitest";
import { thinkLevelFromIndex, thinkLevelIndex } from "./think-level";

describe("thinkLevelIndex", () => {
  it("maps each level and clamps unknown indexes", () => {
    expect(thinkLevelIndex("off")).toBe(0);
    expect(thinkLevelIndex("light")).toBe(1);
    expect(thinkLevelIndex("deep")).toBe(2);
    expect(thinkLevelFromIndex(-4)).toBe("off");
    expect(thinkLevelFromIndex(1.4)).toBe("light");
    expect(thinkLevelFromIndex(9)).toBe("deep");
  });
});
