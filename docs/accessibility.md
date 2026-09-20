# Larra accessibility reference

Keyboard-first notes for VoiceOver on macOS (verified against Larra 0.9.2, Tauri 2 webview). Windows Narrator has not been verified. Gaps are listed at the end.

## Platform

| Item | Value |
|------|--------|
| App | Larra desktop |
| OS | macOS, Windows 10/11 |
| Screen reader | VoiceOver (macOS). Narrator not verified. |
| UI | Custom webview (not native AppKit controls) |

## Landmarks

| Region | How to get there |
|--------|------------------|
| Sidebar | First tab stop after the window; Chat, Shelves, Recipes, Settings. An Update control appears above Settings only when a newer GitHub release was confirmed |
| Main | Next to the sidebar; heading depends on the view |
| Chat composer | Bottom of Chat; `textarea` |
| Document drawer | Dialog-like overlay on Shelves when a row is opened |

Document drawer, source panel, the Reset Larra dialog, Explore other AIs, the AI More info card, the Update window, and the composer Shelf picker trap Tab, move initial focus into the dialog, and close on Escape. Chat announces preparing, writing, and a finished or interrupted answer through a polite live region, and it announces an Online approval when one is waiting. Icon-only Send / Stop / New conversation / Add files / Rename / Download expose `aria-label`. Copy and delete controls are visible without hover.

## Keyboard

| Action | Keys |
|--------|------|
| Move between sidebar and main | `Tab` / `Shift+Tab` |
| Activate a button | `Space` or `Return` |
| Send chat | `Return` (composer; Shift+Return for newline, see ChatView) |
| New conversation | `⌘N` (Windows: `Ctrl+N`) |
| Chat / Shelves / Recipes | `⌘1` / `⌘2` / `⌘3` (Windows: `Ctrl+1`–`3`) |
| Settings | `⌘,` (Windows: `Ctrl+,`) |
| Larger / smaller text | `⌘+` / `⌘-` (Windows: `Ctrl+` / `Ctrl-`). Three steps: Default, Large, Larger |
| Fill a «placeholder» from Shelf files | `↑` `↓` then `Return` or `Tab` (composer, when a list is shown). `Escape` dismisses the list. `Shift+Return` still inserts a newline. During IME composition, `Return` commits the characters and does not send |
| Close drawers, source panel, Reset dialog, Explore other AIs, AI More info, Update window, Shelf picker | `Escape` |
| Open citation | `Return` on a citation chip inside the answer |
| Read earlier Chat messages | `Read more` at the top of a long conversation |
| Move in a long Shelf file table | `↑` `↓` `Home` `End` on a row. Or enable Show all rows for the full native table |

VoiceOver rotor: form controls and buttons. Headings exist in Shelves/Settings; Chat thread list is a list of buttons.

## Known gaps (0.9.2)

- No VoiceOver custom actions for "copy without personal information" beyond the visible button label.
- Windows Narrator has not been verified.

Form fields on Shelves, Recipes, Settings, and Chat (composer, Shelf picker, Reset confirmation) have labels. Icon-only controls expose `aria-label`. Drawers, the Reset dialog, Explore other AIs, the AI More info card, and the Update window trap Tab.

## Settings relevant to AT

House rules and Diagnostics are ordinary text. Settings → Text size has three steps (Default, Large, Larger). `⌘+` and `⌘-` (Windows: `Ctrl+` / `Ctrl-`) move between them. Increasing the OS font size also scales the webview.
