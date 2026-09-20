<script lang="ts">
  import { api, invokeError } from "$lib/api";
  import { t } from "$lib/i18n.svelte";
  import { notifyInvokeError } from "$lib/stores.svelte";
  import { Copy, ShieldCheck, Check } from "@lucide/svelte";

  let { text, subtle = false }: { text: string; subtle?: boolean } = $props();

  let hasPii = $state(false);
  let copied = $state<"plain" | "redacted" | null>(null);

  $effect(() => {
    const current = text;
    if (!current.trim()) {
      hasPii = false;
      return;
    }
    let alive = true;
    api
      .textHasPii(current)
      .then((v) => {
        if (alive) hasPii = v;
      })
      .catch((error) => {
        if (alive) console.error(invokeError(error));
      });
    return () => {
      alive = false;
    };
  });

  async function copyPlain() {
    try {
      await navigator.clipboard.writeText(text);
      copied = "plain";
      setTimeout(() => (copied = null), 1400);
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  async function copyRedacted() {
    try {
      const redacted = await api.redactText(text);
      await navigator.clipboard.writeText(redacted);
      copied = "redacted";
      setTimeout(() => (copied = null), 1400);
    } catch (error) {
      notifyInvokeError(error);
    }
  }
</script>

<div class="flex flex-wrap items-center gap-x-1 gap-y-0.5 {subtle ? 'text-ink-faint' : ''}">
  <button
    type="button"
    class="btn-ghost shrink-0 !px-2 !py-1 !text-[11.5px] whitespace-nowrap"
    onclick={copyPlain}
    title={t("copy.copy")}
  >
    {#if copied === "plain"}<Check
        size={13}
        class="shrink-0 text-emerald-600 dark:text-emerald-400"
        aria-hidden="true"
      />{:else}<Copy size={13} class="shrink-0" aria-hidden="true" />{/if}
    {t("copy.copy")}
  </button>
  {#if hasPii}
    <button
      type="button"
      class="btn-ghost shrink-0 !px-2 !py-1 !text-[11.5px] whitespace-nowrap"
      onclick={copyRedacted}
      title={t("copy.redactTitle")}
    >
      {#if copied === "redacted"}<Check
          size={13}
          class="shrink-0 text-emerald-600 dark:text-emerald-400"
          aria-hidden="true"
        />{:else}<ShieldCheck size={13} class="shrink-0" aria-hidden="true" />{/if}
      {t("copy.copyWithoutPii")}
    </button>
  {/if}
</div>
