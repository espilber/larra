<script lang="ts">
  import type { ChatPrepareStage } from "$lib/api";
  import {
    THINKING_STATUS,
    THINKING_STATUS_MIN_MS,
    advanceThinkingStatus,
    enqueueThinkingLines,
    thinkingStatusLines,
  } from "$lib/thinking-status";
  import { untrack } from "svelte";

  let {
    warming,
    stage,
    hasShelf,
    hasHistory,
    file,
  }: {
    warming: boolean;
    stage: ChatPrepareStage | null;
    hasShelf: boolean;
    hasHistory: boolean;
    file?: string | null;
  } = $props();

  let displayed = $state<string>(THINKING_STATUS.fallback);
  let shownAt = 0;
  let queue: string[] = [];

  $effect(() => {
    const lines = thinkingStatusLines({ warming, stage, hasShelf, hasHistory, file });
    const now = Date.now();
    const next = enqueueThinkingLines(
      untrack(() => ({ displayed, shownAt, queue })),
      lines,
      now,
    );
    displayed = next.displayed;
    shownAt = next.shownAt;
    queue = next.queue;

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = () => {
      if (cancelled) return;
      const step = advanceThinkingStatus(
        { displayed, shownAt, queue },
        Date.now(),
        THINKING_STATUS_MIN_MS,
      );
      displayed = step.displayed;
      shownAt = step.shownAt;
      queue = step.queue;
      if (step.waitMs != null) {
        timer = setTimeout(tick, step.waitMs);
      }
    };

    if (next.queue.length > 0) {
      timer = setTimeout(tick, Math.max(0, THINKING_STATUS_MIN_MS - (now - next.shownAt)));
    }

    return () => {
      cancelled = true;
      if (timer !== undefined) clearTimeout(timer);
    };
  });
</script>

<span class="min-w-0">{displayed}</span>
