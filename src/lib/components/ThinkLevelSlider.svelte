<script lang="ts">
  import type { ThinkLevel } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { setShelfThinkLevel } from "$lib/stores.svelte";
  import {
    THINK_LABELS,
    THINK_LEVELS,
    thinkLevelFromIndex,
    thinkLevelIndex,
  } from "$lib/think-level";

  let {
    shelfId,
    value,
  }: {
    shelfId: string;
    value: ThinkLevel;
  } = $props();

  const inputId = $derived(`think-level-${shelfId}`);
  const hintId = $derived(`think-level-hint-${shelfId}`);
  const fillPct = $derived(`${thinkLevelIndex(value) * 50}%`);

  function onInput(event: Event) {
    const target = event.currentTarget;
    if (!(target instanceof HTMLInputElement)) return;
    void setShelfThinkLevel(shelfId, thinkLevelFromIndex(Number(target.value)));
  }
</script>

<div class="flex w-full min-w-[220px] flex-col gap-2">
  <label class="text-[12px] font-medium text-ink" for={inputId}>
    {t("shelves.lookThrough")}
  </label>
  <input
    id={inputId}
    name={inputId}
    type="range"
    min="0"
    max="2"
    step="1"
    value={thinkLevelIndex(value)}
    aria-valuetext={THINK_LABELS[value]}
    aria-describedby={hintId}
    class="think-slider"
    style="--think-fill: {fillPct}"
    oninput={onInput}
  />
  <div class="grid grid-cols-3 gap-1">
    {#each THINK_LEVELS as level, index (level)}
      <button
        type="button"
        tabindex="-1"
        class="min-w-0 text-[10.5px] {index === 2
          ? 'text-right'
          : index === 1
            ? 'text-center'
            : 'text-left'} {value === level ? 'font-medium text-ink' : 'text-ink-faint'}"
        onclick={() => setShelfThinkLevel(shelfId, level)}
      >
        {THINK_LABELS[level]}
      </button>
    {/each}
  </div>
  <p id={hintId} class="text-[11.5px] text-ink-faint">{t("shelves.thinkHint")}</p>
</div>
