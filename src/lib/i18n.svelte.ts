import ca from "$locales/ca.json";
import cs from "$locales/cs.json";
import da from "$locales/da.json";
import de from "$locales/de.json";
import el from "$locales/el.json";
import en from "$locales/en.json";
import es from "$locales/es.json";
import fi from "$locales/fi.json";
import fr from "$locales/fr.json";
import it from "$locales/it.json";
import ja from "$locales/ja.json";
import nb from "$locales/nb.json";
import nl from "$locales/nl.json";
import pt from "$locales/pt.json";
import sv from "$locales/sv.json";

export const REVIEWED_LOCALES = ["en", "es", "ca"] as const;
export const DRAFT_LOCALES = [
  "pt",
  "fr",
  "ja",
  "de",
  "it",
  "sv",
  "nb",
  "nl",
  "cs",
  "el",
  "da",
  "fi",
] as const;
export const APP_LOCALES = [...REVIEWED_LOCALES, ...DRAFT_LOCALES] as const;
export type AppLocale = (typeof APP_LOCALES)[number];
export type LocalePref = "system" | AppLocale;

const catalogs = {
  en,
  es,
  ca,
  pt,
  fr,
  ja,
  de,
  it,
  sv,
  nb,
  nl,
  cs,
  el,
  da,
  fi,
} as const;

export const i18n = $state({
  locale: "en" as AppLocale,
});

type Vars = Record<string, string | number>;

type NestedKeys<T> = T extends string
  ? never
  : {
      [K in keyof T & string]: T[K] extends string
        ? K
        : T[K] extends object
          ? `${K}.${NestedKeys<T[K]>}`
          : never;
    }[keyof T & string];

export type MessageKey = NestedKeys<typeof en>;

function lookup(source: unknown, key: string): string | undefined {
  let cur: unknown = source;
  for (const part of key.split(".")) {
    if (!cur || typeof cur !== "object") return undefined;
    cur = (cur as Record<string, unknown>)[part];
  }
  return typeof cur === "string" ? cur : undefined;
}

function interpolate(template: string, vars?: Vars): string {
  if (!vars) return template;
  return template.replace(/%\{(\w+)\}/g, (_, name: string) =>
    vars[name] == null ? `%{${name}}` : String(vars[name]),
  );
}

/** Look up a catalog string. Falls back to English, then to the key. */
export function t(key: MessageKey, vars?: Vars): string;
export function t(key: string, vars?: Vars): string;
export function t(key: string, vars?: Vars): string {
  void i18n.locale;
  const local = lookup(catalogs[i18n.locale], key);
  const fallback = i18n.locale === "en" ? undefined : lookup(catalogs.en, key);
  return interpolate(local ?? fallback ?? key, vars);
}

export function applyLocale(locale: AppLocale) {
  i18n.locale = locale;
  if (typeof document !== "undefined") {
    document.documentElement.lang = locale;
  }
}

export function isAppLocale(value: string): value is AppLocale {
  return (APP_LOCALES as readonly string[]).includes(value);
}

export function parseLocalePref(value: unknown): LocalePref {
  if (value === "system") return "system";
  if (typeof value === "string" && isAppLocale(value)) return value;
  return "system";
}

export function parseAppLocale(value: unknown): AppLocale {
  return typeof value === "string" && isAppLocale(value) ? value : "en";
}

export function dateLocale(): string {
  if (typeof document !== "undefined" && document.documentElement.lang) {
    return document.documentElement.lang;
  }
  return i18n.locale;
}

/** Nested string keys in a catalog, for the parity test. Keys starting with `_` are notes. */
export function catalogKeys(source: unknown, prefix = ""): string[] {
  if (typeof source !== "object" || source == null) return [];
  const out: string[] = [];
  for (const [key, value] of Object.entries(source as Record<string, unknown>)) {
    if (key.startsWith("_")) continue;
    const path = prefix ? `${prefix}.${key}` : key;
    if (typeof value === "string") out.push(path);
    else out.push(...catalogKeys(value, path));
  }
  return out;
}

export const catalogsByLocale = catalogs;
