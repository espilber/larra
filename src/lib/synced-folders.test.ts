import { describe, expect, it } from "vitest";
import { countFittingChips, visibleSyncedFolders } from "./synced-folders";

describe("countFittingChips", () => {
  it("shows every chip when they all fit", () => {
    expect(countFittingChips([80, 80, 80], 300, 28, 6)).toBe(3);
  });

  it("reserves room for the more control when some do not fit", () => {
    expect(countFittingChips([80, 80, 80, 80], 220, 28, 6)).toBe(2);
  });

  it("can show only the more control", () => {
    expect(countFittingChips([120, 120], 40, 28, 6)).toBe(0);
  });
});

describe("visibleSyncedFolders", () => {
  it("keeps the selected folder on the row when it would overflow", () => {
    const folders = ["A", "B", "C", "D"];
    expect(visibleSyncedFolders(folders, 2, (name) => name === "D")).toEqual(["A", "D"]);
    expect(visibleSyncedFolders(folders, 2, (name) => name === "B")).toEqual(["A", "B"]);
  });
});
