import { Menu, MenuItem, PredefinedMenuItem } from "@tauri-apps/api/menu";
import { isTextInput } from "./keys";

export type NativeMenuEntry =
  { kind: "separator" } | { kind: "item"; text: string; enabled?: boolean; action: () => void };

export function isTextualTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (isTextInput(target)) return true;
  return !!target.closest(".md-body, .select-text");
}

export async function popupNativeMenu(entries: NativeMenuEntry[]): Promise<void> {
  const items = await Promise.all(
    entries.map((entry) => {
      switch (entry.kind) {
        case "separator":
          return PredefinedMenuItem.new({ item: "Separator" });
        case "item":
          return MenuItem.new({
            text: entry.text,
            enabled: entry.enabled ?? true,
            action: entry.action,
          });
        default: {
          const _exhaustive: never = entry;
          return _exhaustive;
        }
      }
    }),
  );
  const menu = await Menu.new({ items });
  await menu.popup();
}

async function popupEditMenu(): Promise<void> {
  const menu = await Menu.new({
    items: [
      await PredefinedMenuItem.new({ item: "Cut" }),
      await PredefinedMenuItem.new({ item: "Copy" }),
      await PredefinedMenuItem.new({ item: "Paste" }),
      await PredefinedMenuItem.new({ item: "Separator" }),
      await PredefinedMenuItem.new({ item: "SelectAll" }),
    ],
  });
  await menu.popup();
}

/** Hide the webview menu; show a native Edit menu on selectable text. */
export function installNativeContextMenus(): () => void {
  function onContext(event: MouseEvent) {
    if (event.defaultPrevented) return;
    event.preventDefault();
    if (isTextualTarget(event.target)) {
      void popupEditMenu().catch(() => {});
    }
  }
  window.addEventListener("contextmenu", onContext);
  return () => window.removeEventListener("contextmenu", onContext);
}
