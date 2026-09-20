<script lang="ts">
  import { api, downloadHeadline, formatTransferBytes, type DownloadEvent } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { X } from "@lucide/svelte";

  let {
    download,
    cancelable = false,
    note,
    onDark = false,
    skipVerifyLabel,
  }: {
    download: DownloadEvent;
    cancelable?: boolean;
    note?: string;
    onDark?: boolean;
    skipVerifyLabel?: string;
  } = $props();

  function pct(received?: number, total?: number | null): number {
    if (!received || !total) return 0;
    return Math.min(100, Math.round((received / total) * 100));
  }

  const percent = $derived(download.total ? pct(download.received, download.total) : 0);
  const determinate = $derived(!!download.total && download.total > 0);
  const verifyLabel = $derived(skipVerifyLabel ?? t("downloads.skipVerify"));
</script>

<div class="flex flex-col gap-2">
  <div class="flex items-center justify-between text-[13px]">
    <p class="font-medium {onDark ? 'text-white' : 'text-ink'}">{downloadHeadline(download)}</p>
    <p class="tabular-nums {onDark ? 'text-white/55' : 'text-ink-soft'}">
      {formatTransferBytes(download.received)}{download.total
        ? ` / ${formatTransferBytes(download.total)}`
        : ""}
    </p>
  </div>
  <div
    class="h-1.5 overflow-hidden rounded-full {onDark
      ? 'bg-white/10'
      : 'bg-navy-100 dark:bg-white/10'}"
    role="progressbar"
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={determinate ? percent : undefined}
    aria-label={downloadHeadline(download)}
  >
    {#if determinate}
      <div
        class="progress-bar h-full w-full bg-navy-500"
        style="transform: scaleX({percent / 100})"
      ></div>
    {:else}
      <div class="progress-indeterminate" aria-hidden="true"></div>
    {/if}
  </div>
  {#if note}
    <p class="text-[11.5px] {onDark ? 'text-white/45' : 'text-ink-faint'}">{note}</p>
  {/if}
  {#if (download.kind === "model" && download.phase === "verifying") || cancelable}
    <div class="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1">
      {#if download.kind === "model" && download.phase === "verifying"}
        <button
          type="button"
          class="btn-ghost !px-2 !py-1 !text-[11.5px] {onDark
            ? '!text-white/55 hover:!bg-white/8 hover:!text-white'
            : ''}"
          onclick={() => api.downloadSkipVerify(download.id)}
        >
          {verifyLabel}
        </button>
      {/if}
      {#if cancelable}
        <button
          type="button"
          class="btn-ghost !px-2 !py-1 !text-[11.5px] {onDark
            ? '!text-white/55 hover:!bg-white/8 hover:!text-white'
            : ''}"
          onclick={() => api.downloadCancel(download.id)}
        >
          <X size={11} aria-hidden="true" />
          {t("downloads.cancel")}
        </button>
      {/if}
    </div>
  {/if}
</div>
