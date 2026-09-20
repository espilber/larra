/** Visible name for a conversation-only upload Shelf. */
import { t } from "./i18n.svelte";

export function uploadedFilesLabel(): string {
  return t("shelves.uploadedFiles");
}

/** @deprecated Use uploadedFilesLabel(). */
export const UPLOADED_FILES_LABEL = uploadedFilesLabel;

export function isUploadShelf(shelf: { threadId?: string | null } | null | undefined): boolean {
  return Boolean(shelf?.threadId);
}

export function shelfDisplayName(shelf: { name: string; threadId?: string | null }): string {
  return isUploadShelf(shelf) ? uploadedFilesLabel() : shelf.name;
}

export function threadShelfSubtitle(
  thread: { shelfId?: string | null; uploadShelfId?: string | null },
  shelves: { id: string; name: string }[],
): string | null {
  const libraryId =
    thread.shelfId && thread.shelfId !== thread.uploadShelfId ? thread.shelfId : null;
  if (libraryId) {
    return shelves.find((shelf) => shelf.id === libraryId)?.name ?? null;
  }
  if (thread.uploadShelfId) return uploadedFilesLabel();
  return null;
}
