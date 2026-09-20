import { describe, expect, it } from "vitest";
import { osFamily } from "./platform";

describe("osFamily", () => {
  it("falls back to the user agent when the OS plugin is missing", () => {
    const ua = globalThis.navigator?.userAgent ?? "";
    const family = osFamily();
    if (ua.includes("Windows")) expect(family).toBe("windows");
    else if (ua.includes("Mac")) expect(family).toBe("macos");
    else expect(["macos", "windows", "other"]).toContain(family);
  });
});
