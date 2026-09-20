const NON_TEXT_INPUT_TYPES = new Set([
  "button",
  "submit",
  "reset",
  "checkbox",
  "radio",
  "file",
  "color",
  "range",
  "hidden",
  "image",
]);

export function isTextInput(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target instanceof HTMLTextAreaElement) return !target.disabled && !target.readOnly;
  if (target instanceof HTMLInputElement) {
    if (target.disabled || target.readOnly) return false;
    return !NON_TEXT_INPUT_TYPES.has(target.type);
  }
  if (target instanceof HTMLSelectElement) return !target.disabled;
  return target.isContentEditable === true;
}

/**
 * WKWebView beeps when a character key reaches a non-field. Skip Space so
 * focused buttons still activate.
 * https://github.com/tauri-apps/tauri/issues/2626
 */
export function suppressWebviewBeep(event: KeyboardEvent): void {
  if (event.defaultPrevented) return;
  if (event.metaKey || event.ctrlKey || event.altKey) return;
  if (isTextInput(event.target)) return;
  if (event.key.length !== 1 || event.key === " ") return;
  event.preventDefault();
}
