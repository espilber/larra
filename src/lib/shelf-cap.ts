/** Product cap: a Shelf reads this many files, then stops. */
export const SHELF_FILE_CAP = 1_000;

export function shelfCapMessage(queued: number): string {
  if (queued <= 0) {
    return "This Shelf already has 1,000 files. Remove some to add more.";
  }
  if (queued === 1) {
    return "Added 1 file. This Shelf stops at 1,000 files; the rest were left unread.";
  }
  return `Added ${queued.toLocaleString("en-US")} files. This Shelf stops at 1,000 files; the rest were left unread.`;
}

export function skippedLongMessage(n: number): string | null {
  if (n <= 0) return null;
  if (n === 1) {
    return "That file wasn't copied; the path is too long. Add the folder instead, or use a shorter name.";
  }
  return `${n.toLocaleString("en-US")} files weren't copied; their paths are too long. Add the folder instead, or use a shorter name.`;
}

/** Toast for an import or Add folder result. Null when nothing needs saying. */
export function importFeedback(queued: number, atLimit: boolean, skippedLong = 0): string | null {
  const longMsg = skippedLongMessage(skippedLong);
  if (atLimit) {
    const cap = shelfCapMessage(queued);
    return longMsg ? `${cap} ${longMsg}` : cap;
  }
  if (queued === 0) {
    return longMsg ?? "Those files aren't a supported type.";
  }
  if (longMsg && queued === 1) return `Added 1 file. ${longMsg}`;
  if (longMsg) {
    return `Added ${queued.toLocaleString("en-US")} files. ${longMsg}`;
  }
  return null;
}

/** Files accepted but not yet being read. Empty when none. */
export function waitingMessage(n: number): string {
  if (n <= 0) return "";
  if (n === 1) return "1 file waiting";
  return `${n.toLocaleString("en-US")} files waiting`;
}
