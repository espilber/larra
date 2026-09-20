<script lang="ts">
  import type { ChatActivityStep } from "$lib/api";
  import { activityStepLabel } from "$lib/thinking-status";

  let {
    steps,
    live = false,
  }: {
    steps: ChatActivityStep[];
    live?: boolean;
  } = $props();
</script>

<ol role="list" class="flex list-none flex-col border-l border-ink/10 pl-3">
  {#each steps as step, index (`${index}:${step.stage}:${step.file ?? ""}`)}
    {@const current = live && index === steps.length - 1}
    <li class="relative py-0.5 text-[0.75rem] leading-5 {current ? 'text-ink' : 'text-ink-soft'}">
      <span
        class="absolute top-[0.55em] -left-[9.5px] size-1.5 rounded-full {current
          ? 'bg-navy-500'
          : 'bg-navy-300'}"
        aria-hidden="true"
      ></span>
      {activityStepLabel(step, current)}
    </li>
  {/each}
</ol>
