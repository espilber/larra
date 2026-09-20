export function focusTrap(node: HTMLElement) {
  const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;

  function focusable(): HTMLElement[] {
    return [
      ...node.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), textarea, input, select, [tabindex]:not([tabindex="-1"])',
      ),
    ].filter((el) => {
      if (el.hasAttribute("disabled") || el.getClientRects().length === 0) return false;
      if (el instanceof HTMLInputElement && el.type === "hidden") return false;
      if (el.closest("[hidden], [aria-hidden='true']")) return false;
      return true;
    });
  }

  focusable()[0]?.focus();

  function onKey(event: KeyboardEvent) {
    if (event.key !== "Tab") return;
    const items = focusable();
    if (items.length === 0) return;
    const firstEl = items[0]!;
    const lastEl = items[items.length - 1]!;
    if (event.shiftKey && document.activeElement === firstEl) {
      event.preventDefault();
      lastEl.focus();
    } else if (!event.shiftKey && document.activeElement === lastEl) {
      event.preventDefault();
      firstEl.focus();
    }
  }

  node.addEventListener("keydown", onKey);
  return {
    destroy() {
      node.removeEventListener("keydown", onKey);
      if (previous?.isConnected) previous.focus();
    },
  };
}
