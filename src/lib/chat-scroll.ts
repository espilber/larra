const BOTTOM_EPSILON = 2;

export type ChatScroll = ReturnType<typeof createChatScroll>;

export function createChatScroll(viewport: HTMLElement, content: HTMLElement) {
  let following = true;
  let pointerDown = false;
  let pointerWasFollowing = false;
  let pointerMoved = false;
  let frame: number | null = null;
  let destroyed = false;
  let lastTop = viewport.scrollTop;
  let lastBottom = bottom();

  function bottom() {
    return Math.max(0, viewport.scrollHeight - viewport.clientHeight);
  }

  function atBottom() {
    return bottom() - viewport.scrollTop <= BOTTOM_EPSILON;
  }

  function rememberPosition() {
    lastBottom = bottom();
    lastTop = Math.max(0, Math.min(viewport.scrollTop, lastBottom));
  }

  function pause() {
    following = false;
    if (frame !== null) cancelAnimationFrame(frame);
    frame = null;
  }

  function schedule() {
    if (destroyed || !following || pointerDown || frame !== null) return;
    frame = requestAnimationFrame(() => {
      frame = null;
      if (destroyed || !following || pointerDown) return;
      viewport.scrollTop = bottom();
      // Our own scroll event must not change the user's follow/detach choice.
      rememberPosition();
    });
  }

  function follow() {
    if (destroyed) return;
    following = true;
    rememberPosition();
    schedule();
  }

  function onScroll() {
    const nextBottom = bottom();
    const top = Math.max(0, Math.min(viewport.scrollTop, nextBottom));
    if (pointerDown && top !== lastTop && nextBottom >= lastBottom) pointerMoved = true;
    if (top < lastTop - BOTTOM_EPSILON && nextBottom >= lastBottom) {
      pause();
    } else if (!pointerDown && top > lastTop && atBottom()) {
      follow();
    }
    rememberPosition();
  }

  function onWheel(event: WheelEvent) {
    if (event.ctrlKey) return;
    // Detach before the native scroll and before any queued stream frame runs.
    if (event.deltaY < 0) {
      pointerWasFollowing = false;
      pause();
    } else if (event.deltaY > 0) {
      const unit = event.deltaMode === 2 ? viewport.clientHeight : event.deltaMode === 1 ? 16 : 1;
      if (bottom() - viewport.scrollTop <= event.deltaY * unit + BOTTOM_EPSILON) follow();
    }
  }

  function onKey(event: KeyboardEvent) {
    if (event.defaultPrevented || event.altKey) return;
    const target = event.target;
    if (
      target instanceof HTMLElement &&
      target.closest('input, textarea, select, [contenteditable="true"], [role="textbox"]')
    )
      return;
    if (event.key === " " && target instanceof HTMLElement && target.closest("button, a")) return;
    if (event.key === "End" || (event.metaKey && event.key === "ArrowDown")) {
      event.preventDefault();
      follow();
      return;
    }
    if (event.metaKey && event.key !== "ArrowUp") return;
    if (
      ["ArrowUp", "PageUp", "Home"].includes(event.key) ||
      (event.key === " " && event.shiftKey)
    ) {
      pause();
    } else if (["ArrowDown", "PageDown", "End", " "].includes(event.key) && atBottom()) {
      follow();
    }
  }

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0) return;
    pointerWasFollowing = following;
    pointerMoved = false;
    pointerDown = true;
    pause();
  }

  function onPointerUp() {
    if (!pointerDown) return;
    pointerDown = false;
    if ((pointerWasFollowing && !pointerMoved) || atBottom()) follow();
  }

  function preserveAnchor() {
    pause();
    const viewportTop = viewport.getBoundingClientRect().top;
    const anchor = [...content.querySelectorAll<HTMLElement>("[data-chat-message]")].find(
      (node) => node.getBoundingClientRect().bottom > viewportTop,
    );
    const anchorOffset = () => {
      const rect = viewport.getBoundingClientRect();
      const scale = viewport.offsetHeight ? rect.height / viewport.offsetHeight : 1;
      return ((anchor?.getBoundingClientRect().top ?? rect.top) - rect.top) / (scale || 1);
    };
    const anchorTop = anchorOffset();
    const scrollTop = viewport.scrollTop;
    return () => {
      if (destroyed || !anchor?.isConnected || !content.contains(anchor)) return;
      // Only compensate for content prepended above this message. Streaming
      // below it and scrolling while the request is in flight must not move it.
      const movement = viewport.scrollTop - scrollTop;
      viewport.scrollTop += anchorOffset() - anchorTop + movement;
      rememberPosition();
    };
  }

  const resize = new ResizeObserver(schedule);
  resize.observe(content);
  resize.observe(viewport);
  viewport.addEventListener("scroll", onScroll, { passive: true });
  viewport.addEventListener("wheel", onWheel, { passive: true });
  viewport.addEventListener("keydown", onKey);
  viewport.addEventListener("pointerdown", onPointerDown, { passive: true });
  window.addEventListener("pointerup", onPointerUp, { passive: true });
  window.addEventListener("pointercancel", onPointerUp, { passive: true });
  window.addEventListener("blur", onPointerUp);
  schedule();

  return {
    follow,
    preserveAnchor,
    destroy() {
      destroyed = true;
      pause();
      resize.disconnect();
      viewport.removeEventListener("scroll", onScroll);
      viewport.removeEventListener("wheel", onWheel);
      viewport.removeEventListener("keydown", onKey);
      viewport.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("pointerup", onPointerUp);
      window.removeEventListener("pointercancel", onPointerUp);
      window.removeEventListener("blur", onPointerUp);
    },
  };
}
