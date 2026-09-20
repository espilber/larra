/** A bounded range, including overscan, for a fixed-height document table. */
export function virtualWindow(
  count: number,
  scrollTop: number,
  height: number,
  rowHeight: number,
  overscan = 6,
) {
  const size = Math.max(1, rowHeight);
  const capacity = Math.ceil(Math.max(0, height) / size) + overscan * 2;
  const start = Math.max(
    0,
    Math.min(Math.floor(Math.max(0, scrollTop) / size) - overscan, count - capacity),
  );
  const end = Math.min(count, start + capacity);
  return { start, end, before: start * size, after: (count - end) * size };
}
