import { t } from "./i18n.svelte";
import type { TextSize } from "./api";

export const TEXT_SIZES = ["default", "large", "larger"] as const satisfies readonly TextSize[];

export function textSizeLabel(size: TextSize): string {
  switch (size) {
    case "default":
      return t("settings.textDefault");
    case "large":
      return t("settings.textLarge");
    case "larger":
      return t("settings.textLarger");
    default: {
      const _exhaustive: never = size;
      return _exhaustive;
    }
  }
}

export const TEXT_SIZE_LABELS: Record<TextSize, string> = {
  get default() {
    return textSizeLabel("default");
  },
  get large() {
    return textSizeLabel("large");
  },
  get larger() {
    return textSizeLabel("larger");
  },
};

export const TEXT_SIZE_KEY = "larra.textSize";

export const TEXT_SIZE_SCALE: Record<TextSize, number> = {
  default: 1,
  large: 1.15,
  larger: 1.3,
};

export function parseTextSize(value: unknown): TextSize {
  return value === "large" || value === "larger" ? value : "default";
}

export function textSizeIndex(size: TextSize): number {
  switch (size) {
    case "default":
      return 0;
    case "large":
      return 1;
    case "larger":
      return 2;
    default: {
      const _exhaustive: never = size;
      return _exhaustive;
    }
  }
}

export function textSizeFromIndex(index: number): TextSize {
  const clamped = Math.max(0, Math.min(TEXT_SIZES.length - 1, Math.round(index)));
  return TEXT_SIZES[clamped] ?? "default";
}

export function stepTextSize(size: TextSize, delta: number): TextSize {
  return textSizeFromIndex(textSizeIndex(size) + delta);
}

function defaultStorage(): Storage | undefined {
  try {
    return globalThis.localStorage;
  } catch {
    return undefined;
  }
}

export function persistTextSize(
  size: TextSize,
  storage: Pick<Storage, "setItem"> | undefined = defaultStorage(),
): void {
  if (!storage) return;
  try {
    storage.setItem(TEXT_SIZE_KEY, size);
  } catch {
    // Private mode or a full store should not block Settings.
  }
}

function cssZoomSupported(): boolean {
  return (
    typeof CSS !== "undefined" && typeof CSS.supports === "function" && CSS.supports("zoom", "1.15")
  );
}

async function applyNativeZoom(scale: number): Promise<void> {
  try {
    const { getCurrentWebview } = await import("@tauri-apps/api/webview");
    await getCurrentWebview().setZoom(scale);
  } catch {
    // jsdom, browser preview, or a secondary window.
  }
}

/** Paint the size on this document. Native zoom is used when CSS zoom is missing. */
export function applyTextSize(size: TextSize, root = document.documentElement): void {
  if (size === "default") delete root.dataset.textSize;
  else root.dataset.textSize = size;
  const scale = TEXT_SIZE_SCALE[size];
  if (cssZoomSupported()) {
    root.style.zoom = scale === 1 ? "" : String(scale);
    return;
  }
  root.style.zoom = "";
  void applyNativeZoom(scale);
}

export function restoreTextSize(
  storage: Pick<Storage, "getItem"> | undefined = defaultStorage(),
  root = document.documentElement,
): TextSize {
  const size = parseTextSize(storage?.getItem(TEXT_SIZE_KEY));
  applyTextSize(size, root);
  return size;
}
