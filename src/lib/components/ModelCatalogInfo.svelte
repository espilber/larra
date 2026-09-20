<script lang="ts">
  import { api, catalogHostLabel, type Recommendation } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { focusTrap } from "$lib/focus-trap";
  import { notifyInvokeError } from "$lib/stores.svelte";
  import { ExternalLink, Info } from "@lucide/svelte";

  let {
    rec,
    source = "huggingface",
    onDark = false,
  }: {
    rec: Recommendation;
    source?: string;
    onDark?: boolean;
  } = $props();

  let open = $state(false);
  const panelId = $derived(`model-extra-${rec.reference.replace(/[^a-zA-Z0-9_-]/g, "-")}`);
  const host = $derived(catalogHostLabel(source));

  function toggle() {
    open = !open;
  }

  function close() {
    open = false;
  }

  function openPage() {
    close();
    api.openModelPage(source, rec.reference).catch(notifyInvokeError);
  }
</script>

<div class="relative inline-flex shrink-0">
  <button
    type="button"
    class="btn-ghost relative !p-1 {onDark
      ? 'text-white/55 hover:text-white'
      : 'text-ink-soft hover:text-ink'}"
    aria-label={t("explore.moreAbout", { name: rec.name })}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-controls={open ? panelId : undefined}
    onclick={toggle}
  >
    <span
      class="absolute top-1/2 left-1/2 size-[max(100%,2.75rem)] -translate-1/2 pointer-fine:hidden"
      aria-hidden="true"
    ></span>
    <Info size={14} class="size-3.5" aria-hidden="true" />
  </button>
  {#if open}
    <div class="fixed inset-0 z-20" role="presentation" onclick={close}></div>
    <div
      id={panelId}
      class="absolute top-full left-0 z-30 mt-1.5 w-[16.5rem] rounded-xl border border-paper-line bg-surface p-3 shadow-pop dark:shadow-none"
      role="dialog"
      aria-label={t("explore.moreAbout", { name: rec.name })}
      tabindex="-1"
      use:focusTrap
      onkeydown={(event) => event.key === "Escape" && close()}
    >
      {#if rec.blurb}
        <p class="text-[0.75rem] text-pretty text-ink-soft">{rec.blurb}</p>
      {/if}
      {#if rec.license}
        <p class={rec.blurb ? "mt-2" : ""}>
          <span class="label">{t("explore.license")}</span>
          <span class="mt-0.5 block text-[0.75rem] text-ink">{rec.license}</span>
        </p>
      {/if}
      <button type="button" class="btn-ghost mt-2.5 !px-2 !py-1 !text-[0.75rem]" onclick={openPage}>
        <ExternalLink size={12} class="size-3" aria-hidden="true" />
        {t("explore.moreOn", { host })}
      </button>
    </div>
  {/if}
</div>
