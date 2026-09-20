import { describe, expect, it } from "vitest";
import { shelfListStatus, shelfListStatusLabel } from "./shelf-status";

const base = { files: 8, searchable: 8, reading: 0, waiting: 0, errors: 0 };

describe("shelfListStatus", () => {
  it("is Ready when files are on the Shelf and nothing is in flight", () => {
    expect(shelfListStatus(base)).toBe("ready");
    expect(shelfListStatusLabel("ready")).toBe("Ready");
  });

  it("is Processing while a file is being read", () => {
    expect(shelfListStatus({ ...base, reading: 1, waiting: 4 })).toBe("processing");
    expect(shelfListStatusLabel("processing")).toBe("Processing");
  });

  it("is Syncing when files are waiting and none are being read", () => {
    expect(shelfListStatus({ ...base, waiting: 3 })).toBe("syncing");
    expect(shelfListStatusLabel("syncing")).toBe("Syncing");
  });

  it("hides a badge on an empty Shelf", () => {
    expect(shelfListStatus({ ...base, files: 0, searchable: 0 })).toBeNull();
  });

  it("is Sync error when any file failed", () => {
    expect(shelfListStatus({ ...base, searchable: 7, errors: 1 })).toBe("error");
    expect(shelfListStatus({ ...base, searchable: 0, errors: 2 })).toBe("error");
    expect(shelfListStatusLabel("error")).toBe("Sync error");
  });
});
