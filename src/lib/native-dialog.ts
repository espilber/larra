import { confirm } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n.svelte";

/** OS warning dialog (Ok / Cancel). Falls back to `window.confirm` outside Tauri. */
export async function confirmDanger(message: string, okLabel: string): Promise<boolean> {
  try {
    return await confirm(message, {
      title: t("appName"),
      kind: "warning",
      okLabel,
      cancelLabel: t("dialog.cancel"),
    });
  } catch {
    return window.confirm(message);
  }
}
