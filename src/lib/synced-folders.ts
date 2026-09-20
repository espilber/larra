/** How many chips fit on one line, reserving room for "..." when any are hidden. */
export function countFittingChips(
  widths: number[],
  available: number,
  moreWidth: number,
  gap: number,
): number {
  if (widths.length === 0 || available <= 0) return 0;
  const all = widths.reduce((sum, width) => sum + width + gap, 0);
  if (all <= available) return widths.length;
  const room = available - moreWidth - gap;
  let used = 0;
  let count = 0;
  for (const width of widths) {
    const next = used + width + gap;
    if (next > room) break;
    used = next;
    count += 1;
  }
  return count;
}

/** Keep a selected folder on the row when it would otherwise sit behind "...". */
export function visibleSyncedFolders<T>(
  folders: readonly T[],
  visibleCount: number,
  isSelected: (folder: T) => boolean,
): T[] {
  if (visibleCount <= 0) return [];
  const visible = folders.slice(0, visibleCount);
  if (visible.some(isSelected)) return visible;
  const selected = folders.find(isSelected);
  if (!selected) return visible;
  return [...visible.slice(0, -1), selected];
}
