import { describe, expect, it } from "vitest";
import type { ModelSearchResult } from "$lib/api";
import {
  EXPLORE_PAGE_SIZE,
  chipSortActive,
  columnAriaSort,
  nextExploreSort,
  normalizeExploreQuery,
  parseExploreRepoQuery,
  sortExploreResults,
  visibleExploreCount,
} from "./explore-models";

function hit(name: string, extras: Partial<ModelSearchResult> = {}): ModelSearchResult {
  return {
    id: name,
    name,
    source: "huggingface",
    reference: `org/${name}`,
    ...extras,
  };
}

describe("explore repo paste", () => {
  it("reads owner/repo and Hugging Face page URLs", () => {
    expect(parseExploreRepoQuery("OBLITERATUS/Qwen3.8-27B-OBLITERATED")).toBe(
      "OBLITERATUS/Qwen3.8-27B-OBLITERATED",
    );
    expect(
      parseExploreRepoQuery(
        " https://huggingface.co/OBLITERATUS/Qwen3.8-27B-OBLITERATED/tree/main ",
      ),
    ).toBe("OBLITERATUS/Qwen3.8-27B-OBLITERATED");
    expect(parseExploreRepoQuery("hf.co/unsloth/Qwen3-0.6B-GGUF")).toBe("unsloth/Qwen3-0.6B-GGUF");
    expect(parseExploreRepoQuery("<https://huggingface.co/Qwen/Qwen3-8B>")).toBe("Qwen/Qwen3-8B");
    expect(normalizeExploreQuery("  https://huggingface.co/Qwen/Qwen3-8B  ")).toBe("Qwen/Qwen3-8B");
    expect(parseExploreRepoQuery("Qwen3")).toBeUndefined();
    expect(parseExploreRepoQuery("https://evil.example/owner/repo")).toBeUndefined();
    expect(parseExploreRepoQuery("https://huggingface.co/datasets/owner/repo")).toBeUndefined();
    expect(parseExploreRepoQuery("owner/repo/extra")).toBeUndefined();
  });
});

describe("explore sort", () => {
  it("keeps Best in the catalog order", () => {
    const rows = [
      hit("too-large"),
      hit("official-old-rightsize"),
      hit("official-recent-rightsize"),
    ];
    expect(sortExploreResults(rows, "best").map((r) => r.name)).toEqual([
      "too-large",
      "official-old-rightsize",
      "official-recent-rightsize",
    ]);
  });

  it("orders by newest, smallest file, and most downloads", () => {
    const rows = [
      hit("old-big", { released: "2025-01-01", sizeBytes: 8, downloads: 10 }),
      hit("new-small", { released: "2026-08-01", sizeBytes: 2, downloads: 3 }),
    ];
    expect(sortExploreResults(rows, "released").map((r) => r.name)).toEqual([
      "new-small",
      "old-big",
    ]);
    expect(sortExploreResults(rows, "size").map((r) => r.name)).toEqual(["new-small", "old-big"]);
    expect(sortExploreResults(rows, "downloads").map((r) => r.name)).toEqual([
      "old-big",
      "new-small",
    ]);
    expect(sortExploreResults(rows, "released", "asc").map((r) => r.name)).toEqual([
      "old-big",
      "new-small",
    ]);
    expect(sortExploreResults(rows, "size", "desc").map((r) => r.name)).toEqual([
      "old-big",
      "new-small",
    ]);
    expect(sortExploreResults(rows, "downloads", "asc").map((r) => r.name)).toEqual([
      "new-small",
      "old-big",
    ]);
  });

  it("toggles a column header between default and reverse", () => {
    expect(nextExploreSort("best", "desc", "released")).toEqual({ sort: "released", dir: "desc" });
    expect(nextExploreSort("released", "desc", "released")).toEqual({
      sort: "released",
      dir: "asc",
    });
    expect(nextExploreSort("released", "asc", "released")).toEqual({
      sort: "released",
      dir: "desc",
    });
    expect(nextExploreSort("released", "asc", "size")).toEqual({ sort: "size", dir: "asc" });
    expect(chipSortActive("released", "released", "desc")).toBe(true);
    expect(chipSortActive("released", "released", "asc")).toBe(false);
  });

  it("pages at 50 and never past the end", () => {
    expect(EXPLORE_PAGE_SIZE).toBe(50);
    expect(visibleExploreCount(75, 1)).toBe(50);
    expect(visibleExploreCount(75, 2)).toBe(75);
    expect(visibleExploreCount(12, 1)).toBe(12);
  });

  it("marks the active column sort for the table", () => {
    expect(columnAriaSort("size", "size")).toBe("ascending");
    expect(columnAriaSort("size", "size", "desc")).toBe("descending");
    expect(columnAriaSort("downloads", "downloads")).toBe("descending");
    expect(columnAriaSort("released", "best")).toBe("none");
  });
});
