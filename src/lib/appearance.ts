import { getCurrentWindow } from "@tauri-apps/api/window";

export type ColorScheme = "light" | "dark";

const listeners = new Set<(scheme: ColorScheme) => void>();
let current: ColorScheme = "light";

export function colorScheme(): ColorScheme {
  return current;
}

export function onColorSchemeChange(fn: (scheme: ColorScheme) => void): () => void {
  listeners.add(fn);
  return () => {
    listeners.delete(fn);
  };
}

function apply(scheme: ColorScheme) {
  current = scheme;
  const root = document.documentElement;
  root.dataset.theme = scheme;
  root.style.colorScheme = scheme;
  for (const fn of listeners) fn(scheme);
}

/** Match `prefers-color-scheme` before Tauri's window theme is available. */
export function applyColorSchemeFromSystem(): void {
  const dark = globalThis.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
  apply(dark ? "dark" : "light");
}

/** Follow the window theme (OS appearance) and keep CSS in sync. */
export async function watchWindowTheme(): Promise<() => void> {
  try {
    const window = getCurrentWindow();
    const theme = await window.theme();
    apply(theme === "dark" ? "dark" : "light");
    return await window.onThemeChanged(({ payload }) => {
      apply(payload === "dark" ? "dark" : "light");
    });
  } catch {
    return () => {};
  }
}
