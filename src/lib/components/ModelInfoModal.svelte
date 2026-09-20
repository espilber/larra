<script lang="ts">
  import {
    api,
    catalogHostLabel,
    formatBytes,
    formatCount,
    type ModelSearchResult,
  } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { dialogPanel, overlay } from "$lib/motion";
  import { focusTrap } from "$lib/focus-trap";
  import { notifyInvokeError } from "$lib/stores.svelte";
  import { Download, ExternalLink, X } from "@lucide/svelte";

  let {
    result,
    installing = false,
    onClose,
    onInstall,
  }: {
    result: ModelSearchResult;
    installing?: boolean;
    onClose: () => void;
    onInstall: () => void;
  } = $props();

  type InfoField =
    | "publisher"
    | "catalog"
    | "reference"
    | "file"
    | "size"
    | "downloads"
    | "released"
    | "license"
    | "computer";

  function catalogLabel(source: string): string {
    switch (source) {
      case "huggingface":
        return t("explore.huggingface");
      case "ollama":
        return t("explore.ollama");
      case "huggingface+ollama":
        return t("explore.hfAndOllama");
      default:
        return source.replace("+", " + ");
    }
  }

  function infoFieldLabel(field: InfoField): string {
    switch (field) {
      case "publisher":
        return t("explore.infoPublisher");
      case "catalog":
        return t("explore.infoCatalog");
      case "reference":
        return t("explore.infoReference");
      case "file":
        return t("explore.infoFile");
      case "size":
        return t("explore.infoSize");
      case "downloads":
        return t("explore.infoDownloads");
      case "released":
        return t("explore.infoReleased");
      case "license":
        return t("explore.infoLicense");
      case "computer":
        return t("explore.infoComputer");
      default: {
        const _never: never = field;
        return _never;
      }
    }
  }

  function computerFit(fits?: boolean): string | null {
    switch (fits) {
      case true:
        return t("explore.fitsComputer");
      case false:
        return t("explore.tooLargeComputer");
      case undefined:
        return null;
      default: {
        const _never: never = fits;
        return _never;
      }
    }
  }

  const rows = $derived(
    (
      [
        ["publisher", result.publisher],
        ["catalog", catalogLabel(result.source)],
        ["reference", result.reference],
        ["file", result.file],
        ["size", result.sizeBytes != null ? formatBytes(result.sizeBytes) : null],
        [
          "downloads",
          result.downloads != null && result.downloads > 0 ? formatCount(result.downloads) : null,
        ],
        ["released", result.released],
        ["license", result.license],
        ["computer", computerFit(result.fits)],
      ] satisfies [InfoField, string | null | undefined][]
    ).filter((row): row is [InfoField, string] => typeof row[1] === "string" && row[1].length > 0),
  );
</script>

<div
  class="fixed inset-0 z-50 flex items-center justify-center bg-navy-950/25 p-6 dark:bg-black/50"
  role="dialog"
  aria-modal="true"
  aria-labelledby="model-info-title"
  id="model-info-dialog"
  tabindex="-1"
  use:focusTrap
  transition:overlay
  onclick={(e) => e.target === e.currentTarget && onClose()}
  onkeydown={(e) => {
    if (e.key !== "Escape") return;
    e.stopPropagation();
    onClose();
  }}
>
  <div class="card w-full max-w-[440px] shadow-pop dark:shadow-none" in:dialogPanel>
    <div class="flex items-start justify-between gap-3 px-5 pt-5 pb-3">
      <div class="min-w-0">
        <h2 id="model-info-title" class="text-[16px] font-semibold break-words text-ink">
          {result.name}
        </h2>
        {#if result.official}
          <span
            class="mt-2 inline-flex rounded-full bg-navy-100 px-2 py-0.5 text-[10px] font-semibold text-navy-800 dark:bg-white/10 dark:text-navy-100"
          >
            {t("explore.official")}
          </span>
        {/if}
      </div>
      <button
        type="button"
        class="btn-ghost !p-1.5"
        aria-label={t("explore.close")}
        onclick={onClose}
      >
        <X size={15} aria-hidden="true" />
      </button>
    </div>
    <dl class="grid grid-cols-2 gap-x-4 gap-y-2.5 px-5 pb-4 text-[12.5px]">
      {#each rows as [field, value] (field)}
        <div class={field === "reference" || field === "file" ? "col-span-2" : ""}>
          <dt class="label">{infoFieldLabel(field)}</dt>
          <dd class="mt-0.5 break-all text-ink">{value}</dd>
        </div>
      {/each}
    </dl>
    <div class="flex items-center justify-end gap-2 border-t border-paper-line px-5 py-3">
      <button
        type="button"
        class="btn-ghost mr-auto !px-2 !py-1 !text-[12.5px]"
        onclick={() => api.openModelPage(result.source, result.reference).catch(notifyInvokeError)}
      >
        <ExternalLink size={12} aria-hidden="true" />
        {t("explore.moreOn", { host: catalogHostLabel(result.source) })}
      </button>
      <button type="button" class="btn-outline" onclick={onClose}>{t("explore.close")}</button>
      <button
        type="button"
        class="btn-primary"
        onclick={onInstall}
        disabled={installing || result.fits === false}
      >
        <Download size={13.5} aria-hidden="true" />
        {t("explore.install")}
      </button>
    </div>
  </div>
</div>
