/** @vitest-environment jsdom */

import { describe, expect, it } from "vitest";
import { formatCardSummary, isSafeHref, renderMarkdown } from "./markdown";
import type { SourcePassage } from "./api";

const source: SourcePassage = {
  sid: "S1",
  documentId: "d1",
  shelfId: "s1",
  title: "Contract",
  path: "/tmp/contract.md",
  body: "ninety days",
  pageStart: 2,
  score: 1,
};

describe("renderMarkdown", () => {
  it("strips javascript: hrefs", () => {
    const html = renderMarkdown("[x](javascript:alert(1))");
    expect(html).not.toMatch(/javascript:/i);
    expect(html).not.toContain("href=");
  });

  it("adds rel=noopener noreferrer on http links", () => {
    const html = renderMarkdown("[docs](https://example.com/a)");
    expect(html).toContain('href="https://example.com/a"');
    expect(html).toContain('rel="noopener noreferrer"');
    expect(html).toContain('target="_blank"');
  });

  it("does not execute raw HTML", () => {
    const html = renderMarkdown('<img src=x onerror="alert(1)"><script>alert(1)</script>');
    expect(html).not.toContain("<img");
    expect(html).not.toContain("<script");
    expect(html).not.toContain("onerror");
  });

  it("only turns real citation ids into buttons", () => {
    const html = renderMarkdown("See [S1] and [S99].", [source]);
    expect(html).toContain('data-sid="S1"');
    expect(html).toContain('class="cite-chip"');
    expect(html).not.toContain('data-sid="S99"');
    expect(html).toContain("[S99]");
  });

  it("drops forged cite buttons in model HTML", () => {
    const html = renderMarkdown(
      '<button class="cite-chip" data-sid="S1">S1</button><button>steal</button>',
    );
    expect(html).not.toContain(">steal<");
  });
});

describe("formatCardSummary", () => {
  it("reopens flattened ATX headings and splits outline titles from body", () => {
    const text = formatCardSummary(
      "# Chapter one ## Opening The clause lasts ninety days. Notice is written. ## Next A later section follows. ## Open A few items remain.",
      ["Chapter one", "Opening", "Next", "Open"],
    );
    expect(text).toContain("# Chapter one\n\n");
    expect(text).toContain("## Opening\n\nThe clause lasts ninety days.");
    expect(text).toContain("## Next\n\nA later section follows.");
    expect(text).toContain("## Open\n\nA few items remain.");

    const html = renderMarkdown(text);
    expect(html).toMatch(/<h1[^>]*>Chapter one<\/h1>/);
    expect(html).toMatch(/<h2[^>]*>Opening<\/h2>/);
    expect(html).toContain("The clause lasts ninety days.");
    expect(html).not.toContain("# Chapter");
  });

  it("leaves already-broken markdown alone", () => {
    const source = "# Chapter one\n\n## Opening\n\nThe clause lasts ninety days.";
    expect(formatCardSummary(source, ["Chapter one", "Opening"])).toBe(source);
  });
});

describe("isSafeHref", () => {
  it("allows http(s) and mailto only", () => {
    expect(isSafeHref("https://example.com")).toBe(true);
    expect(isSafeHref("mailto:a@b.c")).toBe(true);
    expect(isSafeHref("javascript:alert(1)")).toBe(false);
    expect(isSafeHref("data:text/html,x")).toBe(false);
  });
});

it("keeps block rendering equivalent for tables, references and fenced code", async () => {
  const { createBlockRenderer } = await import("./markdown");
  const render = createBlockRenderer();
  const prefix = "# Stable heading\n\nFirst paragraph.\n\n";
  const first = render(prefix);
  const next = render(
    prefix +
      "| A | B |\n|---|---|\n| 1 | 2 |\n\n```js\nconst a = 1;\n```\n\n[Site][ref]\n\n[ref]: https://example.com\n",
  );
  expect(next[0]).toBe(first[0]);
  expect(next.join("")).toContain("<table>");
  expect(next.join("")).toContain('href="https://example.com"');
  expect(next.join("")).toContain("const a = 1;");
});
