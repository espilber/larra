import type { MenuAction } from "./api";
import { isMac } from "./platform";

export function isModKey(event: KeyboardEvent): boolean {
  return isMac() ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

function textSizeAction(event: KeyboardEvent): MenuAction | null {
  switch (event.key) {
    case "+":
    case "=":
    case "Add":
      return "text-larger";
    case "-":
    case "_":
    case "Subtract":
      return "text-smaller";
    default:
      return null;
  }
}

export function shortcutAction(event: KeyboardEvent): MenuAction | null {
  if (!isModKey(event) || event.altKey || event.repeat) return null;
  const sizeAction = textSizeAction(event);
  if (sizeAction) return sizeAction;
  if (event.shiftKey) return null;
  switch (event.key) {
    case "n":
    case "N":
      return "new-conversation";
    case "1":
      return "view-chat";
    case "2":
      return "view-shelves";
    case "3":
      return "view-recipes";
    case ",":
      return "view-settings";
    default:
      return null;
  }
}

export function parseMenuAction(action: string): MenuAction | null {
  switch (action) {
    case "new-conversation":
    case "view-chat":
    case "view-shelves":
    case "view-recipes":
    case "view-settings":
    case "text-larger":
    case "text-smaller":
      return action;
    default:
      return null;
  }
}
