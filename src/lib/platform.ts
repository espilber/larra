import { platform } from "@tauri-apps/plugin-os";

export type OsFamily = "macos" | "windows" | "other";

function fromUserAgent(): OsFamily {
  const ua = globalThis.navigator?.userAgent ?? "";
  if (ua.includes("Windows")) return "windows";
  if (ua.includes("Mac")) return "macos";
  return "other";
}

/** macOS or Windows when the OS plugin is available; user agent otherwise. */
export function osFamily(): OsFamily {
  try {
    const name = platform();
    if (name === "macos") return "macos";
    if (name === "windows") return "windows";
    return "other";
  } catch {
    return fromUserAgent();
  }
}

export function isMac(): boolean {
  return osFamily() === "macos";
}

export function isWindows(): boolean {
  return osFamily() === "windows";
}
