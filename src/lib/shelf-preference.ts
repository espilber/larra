/** Last Shelf the person picked in Chat, including an explicit No Shelf. */

export type ShelfPreference = string | null | undefined;

export const PREFERRED_SHELF_KEY = "larra.preferredShelfId";

/** Shelf for a new conversation: last manual pick, else the only Shelf. */
export function shelfForNewConversation(
  preferred: ShelfPreference,
  shelfIds: readonly string[],
): string | null {
  if (preferred === undefined) {
    return shelfIds.length === 1 ? (shelfIds[0] ?? null) : null;
  }
  if (preferred === null) return null;
  return shelfIds.includes(preferred) ? preferred : null;
}

export function loadPreferredShelf(
  storage: Pick<Storage, "getItem"> | undefined = defaultStorage(),
): ShelfPreference {
  if (!storage) return undefined;
  try {
    const raw = storage.getItem(PREFERRED_SHELF_KEY);
    if (raw === null) return undefined;
    if (raw === "") return null;
    return raw;
  } catch {
    return undefined;
  }
}

export function savePreferredShelf(
  id: string | null,
  storage: Pick<Storage, "setItem"> | undefined = defaultStorage(),
): void {
  if (!storage) return;
  try {
    storage.setItem(PREFERRED_SHELF_KEY, id ?? "");
  } catch {
    // Private mode or a full store should not block Chat.
  }
}

function defaultStorage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}
