import type { SourcePassage } from "./api";

export type SourceChip = {
  key: string;
  sids: string[];
  source: SourcePassage;
  showSection: boolean;
  page: string | null;
};

function sidNumber(sid: string): number {
  const n = Number(sid.replace(/^S/i, ""));
  return Number.isFinite(n) ? n : Number.POSITIVE_INFINITY;
}

function locationKey(source: SourcePassage): string {
  if (source.anchor)
    return `${source.documentId}\0${source.anchor.hash}\0${source.anchor.startChar ?? source.sid}\0${source.anchor.endChar ?? ""}`;
  if (source.pageStart == null && !source.section) {
    return `sid:${source.sid}`;
  }
  const doc = source.documentId || source.path;
  return `${doc}\0${source.pageStart ?? ""}\0${source.section ?? ""}`;
}

function pageKey(source: SourcePassage): string {
  return `${source.documentId || source.path}\0${source.pageStart ?? ""}`;
}

function pageLabel(source: SourcePassage): string | null {
  if (!source.pageStart) return null;
  if (source.pageEnd && source.pageEnd !== source.pageStart) {
    return `p. ${source.pageStart}–${source.pageEnd}`;
  }
  return `p. ${source.pageStart}`;
}

/** Group cited excerpts that open the same place. Inline [S1] / [S2] stay as-is. */
export function groupSourceChips(sources: SourcePassage[]): SourceChip[] {
  const groups = new Map<string, SourcePassage[]>();
  const order: string[] = [];
  for (const source of sources) {
    if (!source.sid) continue;
    const key = locationKey(source);
    if (!groups.has(key)) {
      order.push(key);
      groups.set(key, []);
    }
    groups.get(key)!.push(source);
  }

  const sectionsOnPage = new Map<string, Set<string>>();
  for (const key of order) {
    const first = groups.get(key)?.[0];
    if (!first) continue;
    const page = pageKey(first);
    const sections = sectionsOnPage.get(page) ?? new Set<string>();
    sections.add(first.section ?? "");
    sectionsOnPage.set(page, sections);
  }

  const chips: SourceChip[] = [];
  for (const key of order) {
    const list = groups.get(key);
    if (!list?.length) continue;
    const sids = [...new Set(list.map((source) => source.sid))].sort(
      (a, b) => sidNumber(a) - sidNumber(b),
    );
    const source = [...list].sort((a, b) => sidNumber(a.sid) - sidNumber(b.sid))[0];
    if (!source) continue;
    const showSection =
      Boolean(source.section?.trim()) && (sectionsOnPage.get(pageKey(source))?.size ?? 0) > 1;
    chips.push({ key, sids, source, showSection, page: pageLabel(source) });
  }
  return chips;
}

export function sourceChipLabel(chip: SourceChip): string {
  const parts = [chip.sids.join(" · "), chip.source.title];
  if (chip.showSection && chip.source.section) parts.push(chip.source.section);
  if (chip.page) parts.push(chip.page);
  return parts.filter(Boolean).join(" ");
}
