# UI

Tokens and shared classes for the desktop app. Source of truth: [`src/app.css`](../src/app.css). Tailwind v4 `@theme` exposes the same names as utilities (`bg-paper`, `text-ink`, `rounded-xl`).

The `navy-*` scale is product green, not blue. The name is historical. Mint aliases point at the same greens. Amber is orange and is for in-progress only.

Do not invent hex in a view. Add a token, or reuse one of these.

## Color

### Product green (`navy` / `mint`)

| Token | Hex | Use |
|-------|-----|-----|
| `navy-50` | `#e8f8f3` | Wash behind a selected row or hint |
| `navy-100` | `#e0f9ee` | Soft fill (`mint-light`) |
| `navy-200` | `#b0efd8` | Soft mint (`mint-soft`) |
| `navy-300` | `#7ed9c0` | Mid mint, slider ticks |
| `navy-400` | `#3ddeba` | Bright mint on dark |
| `navy-500` | `#05d9ab` | Primary fill (`mint`) |
| `navy-600` | `#04c49a` | Primary hover (`mint-hover`) |
| `navy-700` | `#4c744e` | Ink on mint (`ready-ink`) |
| `navy-800` / `900` / `950` | `#103639` | Rail, splash, user chat bubble |

Aliases: `mint`, `mint-hover`, `mint-light`, `mint-soft`, `mint-secondary` (`#aaf1ac`, same as `ready`).

### Paper and ink

| Token | Light | Dark | Use |
|-------|-------|------|-----|
| `paper` | `#f8f8f8` | `#0b1f21` | Window behind cards |
| `paper-soft` | `#f6f7f8` | `#0e292c` | Hover row, code chip |
| `paper-line` | `#d9d9d9` | `#1a4a4e` | Hairline borders |
| `surface` | `#ffffff` | `#14383b` | Cards, inputs, bubbles |
| `ink` | `#0f0b0a` | `#f8f8f8` | Body text |
| `ink-soft` | `#606062` | `#c5d4d2` | Secondary |
| `ink-faint` | `#a0a0a2` | `#8aa09c` | Labels, placeholders |
| `rail-idle` | `#a3acad` | (same) | Inactive sidebar icons |

Dark follows the OS (`html[data-theme="dark"]`, or `prefers-color-scheme` when theme is unset). See [`src/lib/appearance.ts`](../src/lib/appearance.ts). Shadows drop out in dark.

### Status

| Token | Hex | Use |
|-------|-----|-----|
| `ready` / `ready-ink` | `#aaf1ac` / `#4c744e` | Ready badges |
| `amber-350` | `#ffc68a` | In-progress badge fill |
| `amber-450` | `#ff9302` | Pulse dots, Update rail |
| `amber-550` | `#d46d12` | In-progress badge text |
| `danger` | `#e7112d` | Destructive button |
| `danger-hover` | `#c40e26` | Destructive hover |

Shelf list badges live in [`src/lib/shelf-status.ts`](../src/lib/shelf-status.ts): Ready (mint), Processing / Syncing (amber), Sync error (`#F4C2C8` / `#C20F27`). Form errors in a dialog may use `text-red-700` / `dark:text-red-400`.

Amber is not a button color. `btn-amber` is a leftover name; it paints the same mint as `btn-primary`.

## Buttons

Shared classes in `app.css`. All five are **pills** (`rounded-full`), 36px tall (`h-9`), 13px medium, 24px horizontal padding, 6px icon gap. Focus: `outline-2 outline-offset-2 outline-navy-800`. Disabled: 45% opacity, except primary/amber which switch to a mint wash and stay readable.

| Class | Look | When |
|-------|------|------|
| `btn-primary` | Mint fill, ink `#0f0b0a` | The one action on a screen (Save, Create, Install) |
| `btn-amber` | Same as primary | Send, Continue, empty-state Install. Prefer `btn-primary` for new work |
| `btn-outline` | White card, `paper-line` border | Cancel, secondary, "Add folder" |
| `btn-ghost` | No chrome; `ink-soft`; 12px padding | Dismiss, icon rows, "Later" |
| `btn-danger` | Red fill, white type | Reset Larra and other deletes that need a confirm |

Add `btn-icon` for a 36×36 circle (Send, Stop). Ghost icon buttons usually override to `!p-1.5` instead.

Press: primary/amber scale to 0.98 in 80ms. Honor `prefers-reduced-motion`.

Do not square a button. Do not put a second mint button next to the first; the neighbor is outline or ghost.

## Other shapes

| Class / place | Radius | Notes |
|---------------|--------|-------|
| `.card` | `rounded-2xl` (16px) | `paper-line` border, `surface`, `shadow-card` |
| `.input` | `rounded-lg` (10px) | Focus: `navy-500` border + `navy-100` ring |
| `.chip` | full pill | 12px medium; add your own fill/border |
| Rail nav item | `rounded-xl` | 60× wide, icon 19, label 10px |
| Chat composer | card, `!rounded-2xl` | Lives in [`ChatComposer.svelte`](../src/lib/components/ChatComposer.svelte) |
| User bubble | `rounded-2xl rounded-br-md` | `navy-900`, white type |
| Assistant bubble | `rounded-2xl rounded-tl-md` | Surface card + shadow |
| Thread list pane | `rounded-2xl` | 16.5rem when open |
| Menus / typeahead | `rounded-xl` | Surface + `shadow-pop` |
| Dialog / drawer | `.card` | Overlay `navy-950/25` (dark: `black/50`) |
| Onboarding pane | `rounded-3xl` | On `navy-950`; type is white |
| App icon in-app | `rounded-[22%]` | Matches the Dock / Start mark |
| Conversation face | full circle | `sm` 32px, `hero` 94px |

Token radii: `--radius-lg` 10px, `--radius-xl` 16px. Shadows: `shadow-card` and `shadow-pop` are the same (`0 2px 9px` ink at 10%). Dark mode sets both to none.

Drop targets: dashed `navy-500` border, `navy-100/50` wash, `rounded-2xl`.

## Type

System UI only (`ui-sans-serif`, then the OS stack). Mono for Diagnostics and confirm-`DELETE`. Settings → Text size scales the window in three steps (default, 1.15, 1.3) via `html[data-text-size]`.

| Role | Size | Weight |
|------|------|--------|
| Page title (Settings, Recipes) | 22px | semibold |
| Onboarding title | 28px | bold |
| Section title | 15–16px | semibold |
| Body / chat / markdown | 13.5–13.8px | regular, line ~1.62 |
| Button | 13px | medium |
| Secondary / help | 12–12.5px | regular, `ink-soft` |
| Chip | 12px | medium |
| `.label` | 11px | semibold, uppercase, wide tracking, `ink-faint` |
| Rail label | 10px | medium |

Selection wash: navy-500 at 35%. Body is `user-select: none`; text fields, `[contenteditable]`, and `.md-body` opt back in.

## Chrome

- Sidebar is 76px of `navy-950`. Active item: mint type on `navy-500/15`. Idle: `rail-idle`, hover toward white.
- Main pane is `paper`.
- Settings / Recipes content caps at 760px.
- macOS: 46px drag region over the rail so traffic lights sit on the green.

Icons: Lucide (`@lucide/svelte`). 12–15px in buttons, 17–19px in the rail and onboarding cards.

## Motion

Default transition is 100ms. Easing `--ease-out-soft`: `cubic-bezier(0.32, 0.72, 0, 1)`. Shared transitions live in [`src/lib/motion.ts`](../src/lib/motion.ts) (`overlay`, `drawerPanel`, `dialogPanel`, `sheetPanel`, `installCard`, `accordion`). View fade is 120ms. Progress shimmer is 1.4s; it becomes a static mint bar when motion is reduced.

## Toasts

Sonner, top-right. Surface + paper-line. Close control is always visible, top-right of the toast. Offset 52px on macOS (title bar), 16px on Windows.

## Copy and a11y

User-facing English follows the House style in [CONTRIBUTING.md](../CONTRIBUTING.md) (outcome, not machinery; AI over model). Keyboard and VoiceOver: [accessibility.md](accessibility.md). New controls need a name: visible label, or `aria-label` on icon-only buttons.

The main window supports 640 × 540 logical pixels. Below 900 pixels wide, Chat opens its conversation list in a modal drawer, and Shelf and Recipe cards stack vertically. Source panels use the available height. Preserve keyboard focus when opening or closing the drawer.
