import { describe, expect, it } from "vitest";
import {
  findPlaceholders,
  fileListQuery,
  isFileNameSlot,
  pinFileNames,
  placeholderAt,
  previewParts,
  promptNeedsShelf,
  recipeNeedsShelf,
  replacePlaceholder,
} from "./placeholders";

describe("findPlaceholders", () => {
  it("finds each «slot»", () => {
    const slots = findPlaceholders("Compare «document A» and «document B»");
    expect(slots.map((slot) => slot.inner)).toEqual(["document A", "document B"]);
  });
});

describe("placeholderAt", () => {
  it("returns the slot under the cursor", () => {
    const text = "Brief of «document name» please";
    expect(placeholderAt(text, 0)).toBeNull();
    expect(placeholderAt(text, 9)?.inner).toBe("document name");
    expect(placeholderAt(text, 24)).toBeNull();
  });
});

describe("isFileNameSlot", () => {
  it("treats document and file hints as file slots", () => {
    expect(isFileNameSlot("")).toBe(true);
    expect(isFileNameSlot("document name")).toBe(true);
    expect(isFileNameSlot("document A")).toBe(true);
    expect(isFileNameSlot("file")).toBe(true);
    expect(isFileNameSlot("paste the message here")).toBe(false);
    expect(isFileNameSlot("language")).toBe(false);
    expect(isFileNameSlot("topic")).toBe(false);
  });
});

describe("fileListQuery", () => {
  it("shows all files for a file-name hint, filters a typed token, and skips free text", () => {
    expect(fileListQuery("document name")).toBe("");
    expect(fileListQuery("notes.pdf")).toBe("notes.pdf");
    expect(fileListQuery("  Q3  ")).toBe("q3");
    expect(fileListQuery("paste the notes here")).toBeNull();
    expect(fileListQuery("campaign, product or promotion")).toBeNull();
  });
});

describe("replacePlaceholder", () => {
  it("swaps one slot for a file name", () => {
    const text = "Brief of «document name» please";
    const slot = findPlaceholders(text)[0]!;
    expect(replacePlaceholder(text, slot, "plan.pdf")).toBe("Brief of plan.pdf please");
  });
});

describe("pinFileNames", () => {
  it("fills file-name slots in order and leaves extras out", () => {
    expect(
      pinFileNames("Compare «document A» and «document B»", ["one.pdf", "two.pdf", "three.pdf"]),
    ).toBe("Compare one.pdf and two.pdf");
  });

  it("does not fill paste or other free-text slots", () => {
    const paste = "Draft a reply.\n\n«paste the message here»";
    expect(pinFileNames(paste, ["notes.pdf"])).toBe(paste);
    expect(pinFileNames("Translate into «language».\n\n«paste the text here»", ["a.pdf"])).toBe(
      "Translate into «language».\n\n«paste the text here»",
    );
    expect(pinFileNames("What does this Shelf say about «topic»?", ["a.pdf"])).toBe(
      "What does this Shelf say about «topic»?",
    );
  });

  it("fills a document slot and leaves a paste slot", () => {
    expect(
      pinFileNames("Brief of «document name». Notes:\n«paste the notes here»", ["lease.pdf"]),
    ).toBe("Brief of lease.pdf. Notes:\n«paste the notes here»");
  });

  it("puts up to three names in an empty draft", () => {
    expect(pinFileNames("", ["a.pdf", "b.pdf"])).toBe("a.pdf, b.pdf");
    expect(pinFileNames("  ", ["a.pdf", "b.pdf", "c.pdf", "d.pdf"])).toBe("a.pdf, b.pdf, c.pdf");
  });

  it("appends a few names onto an existing draft", () => {
    expect(pinFileNames("Look at", ["a.pdf"])).toBe("Look at a.pdf");
    expect(pinFileNames("Look at ", ["a.pdf"])).toBe("Look at a.pdf");
    expect(pinFileNames("Look at", ["a.pdf", "b.pdf", "c.pdf", "d.pdf"])).toBe("Look at");
  });

  it("does nothing with an empty name list", () => {
    expect(pinFileNames("Hello «x»", [])).toBe("Hello «x»");
  });
});

describe("promptNeedsShelf", () => {
  it("detects Shelf-backed Recipes without matching other uses of the word", () => {
    expect(promptNeedsShelf("Give me a brief of «document name» from this Shelf.")).toBe(true);
    expect(promptNeedsShelf("What do our documents on this Shelf say about «topic»?")).toBe(true);
    expect(promptNeedsShelf("Translate the text below into «language».")).toBe(false);
  });
});

describe("recipeNeedsShelf", () => {
  it("uses the stored flag before the English prompt heuristic", () => {
    expect(recipeNeedsShelf({ prompt: "Hola", needsShelf: true })).toBe(true);
    expect(recipeNeedsShelf({ prompt: "from this Shelf", needsShelf: false })).toBe(true);
    expect(recipeNeedsShelf({ prompt: "Translate this" })).toBe(false);
  });
});

describe("previewParts", () => {
  it("marks «placeholders» for highlight", () => {
    const parts = previewParts("Draft a reply.\n\n«paste the message here»");
    expect(parts.some((part) => part.ph && part.text.includes("paste"))).toBe(true);
    expect(parts.some((part) => !part.ph && part.text.includes("Draft"))).toBe(true);
  });
});
