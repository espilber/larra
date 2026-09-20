/** «placeholders» in Recipe prompts and the Chat composer. */

export interface Placeholder {
  start: number;
  end: number;
  inner: string;
}

const PLACEHOLDER = /«([^»]*)»/g;

export function findPlaceholders(text: string): Placeholder[] {
  const found: Placeholder[] = [];
  const re = new RegExp(PLACEHOLDER.source, "g");
  for (const match of text.matchAll(re)) {
    found.push({
      start: match.index,
      end: match.index + match[0].length,
      inner: match[1] ?? "",
    });
  }
  return found;
}

export function placeholderAt(text: string, cursor: number): Placeholder | null {
  return findPlaceholders(text).find((slot) => slot.start <= cursor && cursor < slot.end) ?? null;
}

/** Empty, or a hint that names a Shelf file («document name», «document A», «file»). */
export function isFileNameSlot(inner: string): boolean {
  const trimmed = inner.trim();
  if (!trimmed) return true;
  return /\bdocuments?\b/i.test(trimmed) || /\bfiles?\b/i.test(trimmed);
}

/**
 * Filter for the Shelf file list. `null` means this slot is paste or another
 * free-text phrase. A typed token is a case-insensitive substring filter.
 */
export function fileListQuery(inner: string): string | null {
  if (isFileNameSlot(inner)) return "";
  const trimmed = inner.trim();
  if (!trimmed || /\s/.test(trimmed)) return null;
  return trimmed.toLowerCase();
}

export function replacePlaceholder(text: string, slot: Placeholder, value: string): string {
  return text.slice(0, slot.start) + value + text.slice(slot.end);
}

/**
 * Dropped or attached file names: fill file-name «slots» in order. Paste and
 * other free-text slots stay put. With no placeholders, put up to three names
 * in an empty draft, or append them to a short existing draft. Extra names
 * are left out so a folder drop does not flood the box.
 */
export function pinFileNames(draft: string, names: string[]): string {
  if (names.length === 0) return draft;
  const all = findPlaceholders(draft);
  const slots = all.filter((slot) => isFileNameSlot(slot.inner));
  if (slots.length > 0) {
    const count = Math.min(slots.length, names.length);
    let result = draft;
    for (let i = count - 1; i >= 0; i--) {
      result = replacePlaceholder(result, slots[i]!, names[i]!);
    }
    return result;
  }
  if (all.length > 0) return draft;
  const clipped = names.slice(0, 3);
  if (!draft.trim()) return clipped.join(", ");
  if (names.length > 3) return draft;
  const extra = clipped.join(", ");
  return /[\s\n]$/.test(draft) ? draft + extra : `${draft} ${extra}`;
}

/** Older saved Recipes that read a Shelf often say "this Shelf". */
export function promptNeedsShelf(prompt: string): boolean {
  return /\bthis shelf\b/i.test(prompt);
}

/** Prefer the stored flag; fall back to the English prompt heuristic for older files. */
export function recipeNeedsShelf(recipe: { needsShelf?: boolean; prompt: string }): boolean {
  return recipe.needsShelf === true || promptNeedsShelf(recipe.prompt);
}

/** Split a preview so «placeholders» can be highlighted. */
export function previewParts(prompt: string): { text: string; ph: boolean }[] {
  const clean = prompt.replace(/\s+/g, " ").trim();
  const parts: { text: string; ph: boolean }[] = [];
  const re = new RegExp(PLACEHOLDER.source, "g");
  let last = 0;
  for (const match of clean.matchAll(re)) {
    if (match.index > last) parts.push({ text: clean.slice(last, match.index), ph: false });
    parts.push({ text: match[0], ph: true });
    last = match.index + match[0].length;
  }
  if (last < clean.length) parts.push({ text: clean.slice(last), ph: false });
  return parts;
}
