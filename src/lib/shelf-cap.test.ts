import { describe, expect, it } from "vitest";
import { importFeedback, shelfCapMessage, waitingMessage } from "./shelf-cap";

describe("shelfCapMessage", () => {
  it("explains a full Shelf when nothing was added", () => {
    expect(shelfCapMessage(0)).toContain("1,000");
    expect(shelfCapMessage(0).toLowerCase()).toContain("remove");
  });

  it("mentions how many were added when some fit", () => {
    expect(shelfCapMessage(1)).toContain("1 file");
    expect(shelfCapMessage(40)).toContain("40 files");
    expect(shelfCapMessage(1000)).toContain("1,000 files");
  });
});

describe("importFeedback", () => {
  it("stays quiet when files were added under the cap", () => {
    expect(importFeedback(12, false)).toBeNull();
  });

  it("explains unsupported types when nothing was queued", () => {
    expect(importFeedback(0, false)).toContain("supported type");
  });

  it("explains a path that is too long", () => {
    expect(importFeedback(0, false, 1)).toContain("too long");
    expect(importFeedback(0, false, 1)).not.toContain("supported type");
    expect(importFeedback(2, false, 3)).toContain("2 files");
    expect(importFeedback(2, false, 3)).toContain("too long");
  });
});

describe("waitingMessage", () => {
  it("names one file and many files", () => {
    expect(waitingMessage(1)).toBe("1 file waiting");
    expect(waitingMessage(1488)).toBe("1,488 files waiting");
  });
});
