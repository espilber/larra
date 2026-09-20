import { describe, expect, it } from "vitest";
import { fileNameFromPath, linkedFolderName, suggestedShelfName } from "./files";

describe("fileNameFromPath", () => {
  it("handles POSIX and Windows paths", () => {
    expect(fileNameFromPath("/Users/example/Notes/plan.pdf")).toBe("plan.pdf");
    expect(fileNameFromPath("C:\\Users\\example\\Notes\\plan.pdf")).toBe("plan.pdf");
    expect(fileNameFromPath("plan.pdf")).toBe("plan.pdf");
  });
});

describe("linkedFolderName", () => {
  it("uses the label, then the last path segment", () => {
    expect(linkedFolderName({ label: "Notes", path: "/Users/example/Notes" })).toBe("Notes");
    expect(linkedFolderName({ path: "/Users/example/Research" })).toBe("Research");
    expect(linkedFolderName({})).toBe("this folder");
  });
});

describe("suggestedShelfName", () => {
  it("uses a folder name and falls back to Files", () => {
    expect(suggestedShelfName(["/Users/example/Research"])).toBe("Research");
    expect(suggestedShelfName(["/Users/example/plan.pdf"])).toBe("Files");
    expect(suggestedShelfName(["/a.pdf", "/b.pdf"])).toBe("Files");
  });
});
