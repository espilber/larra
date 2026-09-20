<script lang="ts">
  import { formatBytes, formatReleased, type Recommendation } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { Download } from "@lucide/svelte";

  let {
    suggestions,
    installing = false,
    heading,
    lede,
    onInstall,
  }: {
    suggestions: Recommendation[];
    installing?: boolean;
    heading: string;
    lede: string;
    onInstall: (rec: Recommendation) => void;
  } = $props();
</script>

{#if suggestions.length > 0}
  <div class="@container">
    <h3 class="text-[0.9375rem] font-semibold text-ink">{heading}</h3>
    <p class="mt-1 max-w-[48ch] text-[0.8125rem] text-pretty text-ink-soft">{lede}</p>
    <ul class="mt-3 grid grid-cols-1 gap-3 @min-[28rem]:grid-cols-2" role="list">
      {#each suggestions as rec, index (rec.reference)}
        {@const featured = index === 0}
        <li
          class="flex flex-col justify-between gap-4 rounded-xl bg-surface p-4 ring-1 ring-navy-950/10 dark:ring-white/10"
        >
          <div class="flex min-w-0 flex-col gap-3">
            <div class="flex flex-col gap-1">
              {#if featured}
                <p class="text-[0.6875rem] font-medium text-navy-700 dark:text-navy-300">
                  {t("explore.bestFit")}
                </p>
              {/if}
              <p class="text-[0.9375rem] font-semibold text-ink">{rec.name}</p>
              <p class="text-[0.8125rem] text-pretty text-ink-soft">{rec.blurb}</p>
            </div>
            <dl class="grid grid-cols-3 gap-x-3 text-[0.6875rem]">
              <div>
                <dt class="font-medium text-ink">{t("explore.from")}</dt>
                <dd class="mt-0.5 text-ink-soft">{rec.provider}</dd>
              </div>
              <div>
                <dt class="font-medium text-ink">{t("explore.download")}</dt>
                <dd class="mt-0.5 text-ink-soft tabular-nums">
                  {t("explore.aboutSize", { size: formatBytes(rec.approxBytes) })}
                </dd>
              </div>
              <div>
                <dt class="font-medium text-ink">{t("explore.infoReleased")}</dt>
                <dd class="mt-0.5 text-ink-soft">{formatReleased(rec.released)}</dd>
              </div>
            </dl>
          </div>
          <div>
            <button
              type="button"
              class="btn-outline w-full !py-1.5 !pl-2 !text-[0.75rem]"
              onclick={() => onInstall(rec)}
              disabled={installing}
            >
              <Download size={16} class="size-4 shrink-0" aria-hidden="true" />
              {t("explore.install")}
            </button>
          </div>
        </li>
      {/each}
    </ul>
  </div>
{/if}
