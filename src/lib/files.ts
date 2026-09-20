import { getCurrentWebview } from "@tauri-apps/api/webview";

export function fileNameFromPath(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] || path;
}

/** Folder chip / confirm copy. Prefer the API label; fall back to the path. */
export function linkedFolderName(linked: { label?: string; path?: string }): string {
  const label = linked.label?.trim();
  if (label) return label;
  if (linked.path) return fileNameFromPath(linked.path);
  return "this folder";
}

export function linkedFolderSourceId(linked: {
  sourceId?: string;
  source_id?: string;
}): string | undefined {
  const id = linked.sourceId || linked.source_id;
  return id || undefined;
}

/** Name a new Shelf from a dropped folder; a lone file becomes "Files". */
export function suggestedShelfName(paths: string[]): string {
  if (paths.length === 1) {
    const base = fileNameFromPath(paths[0]!);
    if (base && !base.includes(".")) return base;
  }
  return "Files";
}

export function listenFileDrop(handlers: {
  onOver: (active: boolean) => void;
  onDrop: (paths: string[]) => void;
}): () => void {
  let unlisten: (() => void) | null = null;
  let cancelled = false;
  getCurrentWebview()
    .onDragDropEvent((event) => {
      switch (event.payload.type) {
        case "enter":
        case "over":
          handlers.onOver(true);
          break;
        case "drop":
          handlers.onOver(false);
          if (event.payload.paths.length > 0) handlers.onDrop(event.payload.paths);
          break;
        case "leave":
          handlers.onOver(false);
          break;
        default: {
          const _exhaustive: never = event.payload;
          void _exhaustive;
        }
      }
    })
    .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
  return () => {
    cancelled = true;
    unlisten?.();
  };
}
