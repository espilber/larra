import { t } from "./i18n.svelte";
import type { ShelfStats } from "./api";

export type ShelfListStatus = "ready" | "processing" | "syncing" | "error";

type StatusStats = Pick<ShelfStats, "files" | "searchable" | "reading" | "errors"> & {
  waiting?: number;
};

/** Badge for a Shelf card: work in flight first, then Ready. Empty Shelves have none. */
export function shelfListStatus(stats: StatusStats): ShelfListStatus | null {
  if (stats.reading > 0) return "processing";
  if ((stats.waiting ?? 0) > 0) return "syncing";
  if (stats.files === 0) return null;
  if (stats.errors > 0) return "error";
  return "ready";
}

export function shelfListStatusLabel(status: ShelfListStatus): string {
  switch (status) {
    case "ready":
      return t("shelves.statusReady");
    case "processing":
      return t("shelves.statusProcessing");
    case "syncing":
      return t("shelves.statusSyncing");
    case "error":
      return t("shelves.statusError");
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}

export function shelfListStatusClass(status: ShelfListStatus): string {
  switch (status) {
    case "ready":
      return "bg-ready text-ready-ink dark:bg-navy-200/20 dark:text-navy-200";
    case "processing":
    case "syncing":
      return "bg-amber-350 text-amber-550";
    case "error":
      return "bg-[#F4C2C8] text-[#C20F27] dark:bg-red-400/10 dark:text-red-400";
    default: {
      const _exhaustive: never = status;
      return _exhaustive;
    }
  }
}
