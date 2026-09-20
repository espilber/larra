import { describe, expect, it } from "vitest";
import type { SourcePassage } from "./api";
import { groupSourceChips, sourceChipLabel } from "./source-chips";

function source(partial: Partial<SourcePassage> & Pick<SourcePassage, "sid">): SourcePassage {
  return {
    documentId: "d1",
    shelfId: "s1",
    title: "B2 First Handbook for teachers for exams",
    path: "/tmp/handbook.pdf",
    score: 1,
    ...partial,
  };
}

describe("groupSourceChips", () => {
  it("merges two excerpts from the same page", () => {
    const chips = groupSourceChips([
      source({ sid: "S2", pageStart: 87, pageEnd: 87 }),
      source({ sid: "S1", pageStart: 87, pageEnd: 87 }),
    ]);
    expect(chips).toHaveLength(1);
    expect(chips[0]?.sids).toEqual(["S1", "S2"]);
    expect(chips[0]?.source.sid).toBe("S1");
    expect(chips[0] && sourceChipLabel(chips[0])).toBe(
      "S1 · S2 B2 First Handbook for teachers for exams p. 87",
    );
  });

  it("keeps different sections on the same page and names them", () => {
    const chips = groupSourceChips([
      source({ sid: "S1", pageStart: 87, section: "Fees" }),
      source({ sid: "S2", pageStart: 87, section: "Dates" }),
    ]);
    expect(chips).toHaveLength(2);
    expect(chips[0]?.showSection).toBe(true);
    expect(chips[1]?.showSection).toBe(true);
    expect(chips[0] && sourceChipLabel(chips[0])).toContain("Fees");
    expect(chips[1] && sourceChipLabel(chips[1])).toContain("Dates");
  });

  it("does not merge two excerpts that have no page or section", () => {
    const chips = groupSourceChips([source({ sid: "S1" }), source({ sid: "S2" })]);
    expect(chips).toHaveLength(2);
    expect(chips[0]?.sids).toEqual(["S1"]);
    expect(chips[1]?.sids).toEqual(["S2"]);
  });

  it("shows a page range when the excerpt spans pages", () => {
    const chips = groupSourceChips([source({ sid: "S1", pageStart: 87, pageEnd: 88 })]);
    expect(chips[0]?.page).toBe("p. 87–88");
  });
});

it("keeps distinct exact passages separate even on the same page", () => {
  const chips = groupSourceChips([
    source({
      sid: "S1",
      pageStart: 2,
      anchor: { hash: "v1", startChar: 100, endChar: 110, quote: "First fact" },
    }),
    source({
      sid: "S2",
      pageStart: 2,
      anchor: { hash: "v1", startChar: 900, endChar: 911, quote: "Second fact" },
    }),
  ]);
  expect(chips).toHaveLength(2);
  expect(chips[1]?.source.anchor?.quote).toBe("Second fact");
});
