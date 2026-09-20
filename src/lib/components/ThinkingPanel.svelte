<script lang="ts">
  import type { ChatActivityStep } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { visibleActivity } from "$lib/thinking-status";
  import { ChevronDown } from "@lucide/svelte";
  import type { Snippet } from "svelte";
  import ChatActivity from "./ChatActivity.svelte";

  let {
    id,
    open,
    onToggle,
    thinking = "",
    activity = [],
    live = false,
    lead,
  }: {
    id: string;
    open: boolean;
    onToggle: () => void;
    thinking?: string | null;
    activity?: ChatActivityStep[] | null;
    live?: boolean;
    lead?: Snippet;
  } = $props();

  const steps = $derived(visibleActivity(activity));
  const thinkingText = $derived((thinking ?? "").trim());
  const hasThinking = $derived(thinkingText.length > 0);
  const hasActivity = $derived(steps.length > 0);
  const visible = $derived(hasThinking || hasActivity);
  const label = $derived(hasThinking ? t("thinking.labelThinking") : t("thinking.labelLooked"));
  const panelId = $derived(`${id}-panel`);
  const compactLabel = $derived(
    open
      ? hasThinking
        ? t("thinking.hideThinking")
        : t("thinking.hideLooked")
      : hasThinking
        ? t("thinking.showThinking")
        : t("thinking.showLooked"),
  );
  const chevronClass = $derived(
    `shrink-0 transition-transform duration-150 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none ${open ? "" : "-rotate-90"}`,
  );
</script>

{#snippet panel()}
  <div id={panelId} class="flex flex-col gap-2.5">
    {#if hasActivity}
      <ChatActivity {steps} {live} />
    {/if}
    {#if hasThinking}
      <p
        class="cursor-text border-l-2 border-paper-line pl-3 text-[0.75rem] leading-5 whitespace-pre-wrap text-ink-faint select-text"
      >
        {thinkingText}
      </p>
    {/if}
  </div>
{/snippet}

{#if lead}
  <div class="flex flex-col gap-2.5">
    <div class="flex items-center gap-2 text-[0.84375rem] text-ink-soft">
      {@render lead()}
      {#if visible}
        <button
          type="button"
          class="relative ml-auto text-ink-faint hover:text-ink-soft focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-500"
          aria-expanded={open}
          aria-controls={panelId}
          aria-label={compactLabel}
          onclick={onToggle}
        >
          <span
            class="absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
            aria-hidden="true"
          ></span>
          <ChevronDown size={11} aria-hidden="true" class={chevronClass} />
        </button>
      {/if}
    </div>
    {#if visible && open}
      {@render panel()}
    {/if}
  </div>
{:else if visible}
  <div class="flex flex-col {open ? 'mb-2.5 gap-2.5' : 'mb-1'}">
    <button
      type="button"
      class="flex items-center gap-1 text-[0.75rem] font-medium whitespace-nowrap text-ink-faint hover:text-ink-soft focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-500"
      aria-expanded={open}
      aria-controls={panelId}
      onclick={onToggle}
    >
      <ChevronDown size={11} aria-hidden="true" class={chevronClass} />
      {label}
    </button>
    {#if open}
      {@render panel()}
    {/if}
  </div>
{/if}
