import { describe, expect, it } from "vitest";
import { APP_LOCALES, DRAFT_LOCALES, catalogKeys, catalogsByLocale, t } from "./i18n.svelte";

describe("catalogs", () => {
  it("keeps the same keys in every shipped catalog", () => {
    const en = catalogKeys(catalogsByLocale.en).sort();
    for (const locale of APP_LOCALES) {
      expect(catalogKeys(catalogsByLocale[locale]).sort(), locale).toEqual(en);
    }
  });

  it("marks draft catalogs for a native-speaker check", () => {
    for (const locale of DRAFT_LOCALES) {
      const review = (catalogsByLocale[locale] as { _review?: string })._review;
      expect(review, locale).toMatch(/AI-generated/);
      expect(review, locale).toContain(`locales/${locale}.json`);
    }
  });

  it("looks up nested keys and interpolates", () => {
    expect(t("errors.shelfExists", { name: "Harbor" })).toBe(
      'A Shelf called "Harbor" already exists.',
    );
  });

  it("translates Chat, Shelves, and Recipes in the sidebar and View menu", () => {
    expect(catalogsByLocale.es.nav.chat).toBe("Chats");
    expect(catalogsByLocale.es.nav.shelves).toBe("Estantes");
    expect(catalogsByLocale.es.nav.recipes).toBe("Recetas");
    expect(catalogsByLocale.ca.nav.chat).toBe("Xat");
    expect(catalogsByLocale.ca.nav.shelves).toBe("Estants");
    expect(catalogsByLocale.ca.nav.recipes).toBe("Receptes");
    expect(catalogsByLocale.es.menu.chat).toBe(catalogsByLocale.es.nav.chat);
    expect(catalogsByLocale.ca.menu.recipes).toBe(catalogsByLocale.ca.nav.recipes);
    for (const locale of APP_LOCALES) {
      const nav = catalogsByLocale[locale].nav;
      const menu = catalogsByLocale[locale].menu;
      expect(menu.chat, locale).toBe(nav.chat);
      expect(menu.shelves, locale).toBe(nav.shelves);
      expect(menu.recipes, locale).toBe(nav.recipes);
    }
  });

  it("keeps this Shelf in English Recipe prompts that read a Shelf", () => {
    const needsShelf = /\bthis shelf\b/i;
    const recipes = catalogsByLocale.en.defaults.recipes;
    const shelfIds = Object.entries(recipes)
      .filter(([, rec]) => needsShelf.test(rec.prompt))
      .map(([id]) => id);
    expect(shelfIds).toEqual([
      "one-page-brief",
      "compare-documents",
      "document-key-terms",
      "policy-qa",
    ]);
  });

  it("does not leave English Shelf in translated catalogs", () => {
    const englishShelf = /\bShelves?\b/;
    for (const locale of APP_LOCALES) {
      if (locale === "en") continue;
      for (const text of catalogStrings(catalogsByLocale[locale])) {
        expect(text, locale).not.toMatch(englishShelf);
      }
    }
    expect(catalogsByLocale.es.nav.shelves).toBe("Estantes");
    expect(catalogsByLocale.es.shelves.emptyTitle).toMatch(/estante/i);
    expect(catalogsByLocale.es.shelves.emptyBody).toMatch(/estante/i);
    expect(catalogsByLocale.es.shelves.createFirst).toMatch(/estante/i);
    expect(catalogsByLocale.ca.nav.shelves).toBe("Estants");
    expect(catalogsByLocale.ca.shelves.emptyTitle).toMatch(/estant/i);
  });
});

function catalogStrings(source: unknown): string[] {
  if (typeof source === "string") return [source];
  if (!source || typeof source !== "object") return [];
  return Object.entries(source as Record<string, unknown>).flatMap(([key, value]) =>
    key.startsWith("_") ? [] : catalogStrings(value),
  );
}
