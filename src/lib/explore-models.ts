import type { ModelSearchResult } from "$lib/api";
import { t } from "$lib/i18n.svelte";

export const EXPLORE_PAGE_SIZE = 50;

const HF_NON_MODEL_ROOTS = new Set([
  "api",
  "blog",
  "chat",
  "collections",
  "datasets",
  "docs",
  "join",
  "learn",
  "login",
  "metrics",
  "models",
  "organizations",
  "papers",
  "pricing",
  "settings",
  "spaces",
  "tasks",
]);

const HF_HUB_PREFIXES = [
  "https://huggingface.co/",
  "http://huggingface.co/",
  "https://www.huggingface.co/",
  "http://www.huggingface.co/",
  "https://hf.co/",
  "http://hf.co/",
  "https://www.hf.co/",
  "http://www.hf.co/",
  "huggingface.co/",
  "www.huggingface.co/",
  "hf.co/",
  "www.hf.co/",
];

function isHfRepoPart(value: string): boolean {
  return value.length > 0 && /^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(value);
}

function hfRepoId(owner: string, repo: string): string | undefined {
  if (HF_NON_MODEL_ROOTS.has(owner.toLowerCase())) return undefined;
  if (!isHfRepoPart(owner) || !isHfRepoPart(repo)) return undefined;
  return `${owner}/${repo}`;
}

function stripExploreQueryNoise(query: string): string {
  return query
    .trim()
    .replace(/^["'`<\u201c]+|["'`>\u201d]+$/g, "")
    .trim();
}

function hfHubPath(query: string): string | undefined {
  const lower = query.toLowerCase();
  for (const prefix of HF_HUB_PREFIXES) {
    if (lower.startsWith(prefix)) {
      return query.slice(prefix.length).split(/[?#]/, 1)[0] ?? "";
    }
  }
  return undefined;
}

/** Hugging Face `owner/repo`, or a huggingface.co / hf.co model page URL. */
export function parseExploreRepoQuery(query: string): string | undefined {
  const raw = stripExploreQueryNoise(query);
  if (!raw) return undefined;
  const path = hfHubPath(raw);
  if (path != null) {
    const [owner, name] = path.split("/").filter(Boolean);
    if (!owner || !name) return undefined;
    return hfRepoId(owner, name);
  }
  const slash = raw.indexOf("/");
  if (slash <= 0 || raw.includes("/", slash + 1)) return undefined;
  return hfRepoId(raw.slice(0, slash), raw.slice(slash + 1));
}

export function normalizeExploreQuery(query: string): string {
  const trimmed = query.trim();
  return parseExploreRepoQuery(trimmed) ?? trimmed;
}

export type ExploreSort = "best" | "released" | "size" | "downloads";
export type ExploreSortDir = "asc" | "desc";
export type ExploreColumn = "released" | "size" | "downloads";

export const EXPLORE_SORTS: { id: ExploreSort }[] = [
  { id: "best" },
  { id: "released" },
  { id: "size" },
  { id: "downloads" },
];

export function exploreSortLabel(sort: ExploreSort): string {
  switch (sort) {
    case "best":
      return t("explore.sortBest");
    case "released":
      return t("explore.sortNewest");
    case "size":
      return t("explore.sortSmallest");
    case "downloads":
      return t("explore.sortDownloads");
    default: {
      const _never: never = sort;
      return _never;
    }
  }
}

export function defaultExploreSortDir(sort: ExploreSort): ExploreSortDir {
  switch (sort) {
    case "best":
    case "released":
    case "downloads":
      return "desc";
    case "size":
      return "asc";
    default: {
      const _never: never = sort;
      return _never;
    }
  }
}

export function nextExploreSort(
  current: ExploreSort,
  currentDir: ExploreSortDir,
  next: ExploreSort,
): { sort: ExploreSort; dir: ExploreSortDir } {
  if (next === "best") return { sort: "best", dir: "desc" };
  if (current === next) {
    return { sort: next, dir: currentDir === "asc" ? "desc" : "asc" };
  }
  return { sort: next, dir: defaultExploreSortDir(next) };
}

export function chipSortActive(chip: ExploreSort, sort: ExploreSort, dir: ExploreSortDir): boolean {
  if (chip === "best") return sort === "best";
  return sort === chip && dir === defaultExploreSortDir(chip);
}

function byName(a: ModelSearchResult, b: ModelSearchResult): number {
  return a.name.localeCompare(b.name);
}

function compareOptionalNumber(
  left: number | undefined,
  right: number | undefined,
  dir: ExploreSortDir,
): number {
  if (left == null && right == null) return 0;
  if (left == null) return 1;
  if (right == null) return -1;
  return dir === "asc" ? left - right : right - left;
}

/** Released / size / downloads. Best keeps the catalog order from search. */
export function sortExploreResults(
  results: ModelSearchResult[],
  sort: ExploreSort,
  dir: ExploreSortDir = defaultExploreSortDir(sort),
): ModelSearchResult[] {
  const copy = results.slice();
  switch (sort) {
    case "best":
      return copy;
    case "released":
      copy.sort((a, b) => {
        const left = a.released ?? "";
        const right = b.released ?? "";
        if (!left && !right) return byName(a, b);
        if (!left) return 1;
        if (!right) return -1;
        const released = dir === "asc" ? left.localeCompare(right) : right.localeCompare(left);
        if (released !== 0) return released;
        return byName(a, b);
      });
      return copy;
    case "size":
      copy.sort((a, b) => {
        const size = compareOptionalNumber(a.sizeBytes, b.sizeBytes, dir);
        if (size !== 0) return size;
        return byName(a, b);
      });
      return copy;
    case "downloads":
      copy.sort((a, b) => {
        const downloads = compareOptionalNumber(a.downloads, b.downloads, dir);
        if (downloads !== 0) return downloads;
        return byName(a, b);
      });
      return copy;
    default: {
      const _exhaustive: never = sort;
      return _exhaustive;
    }
  }
}

export function visibleExploreCount(
  total: number,
  page: number,
  pageSize = EXPLORE_PAGE_SIZE,
): number {
  return Math.min(total, Math.max(page, 1) * pageSize);
}

export function columnAriaSort(
  column: ExploreColumn,
  sort: ExploreSort,
  dir: ExploreSortDir = defaultExploreSortDir(sort),
): "ascending" | "descending" | "none" {
  if (sort !== column) return "none";
  return dir === "asc" ? "ascending" : "descending";
}
