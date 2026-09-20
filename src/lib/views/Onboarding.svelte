<script lang="ts">
  import { tick } from "svelte";
  import { fade } from "svelte/transition";
  import { MessageCircle, LibraryBig, ChefHat, Download, Check, ChevronDown } from "@lucide/svelte";
  import {
    api,
    downloadErrorMessage,
    formatBytes,
    type MachineView,
    type Recommendation,
  } from "$lib/api";
  import DownloadProgress from "$lib/components/DownloadProgress.svelte";
  import ModelCatalogInfo from "$lib/components/ModelCatalogInfo.svelte";
  import { accordion, installCard, motionMs } from "$lib/motion";
  import { app, beginModelInstall, notifyInvokeError, refreshSettings } from "$lib/stores.svelte";
  import { t } from "$lib/i18n.svelte";
  import { shot } from "$lib/shot-control.svelte";
  import mark from "../../assets/R.webp";

  let step = $state<"promise" | "model">(
    shot.onboard === "model" || import.meta.env.VITE_START_ONBOARD === "model"
      ? "model"
      : "promise",
  );
  let machine = $state<MachineView | null>(null);
  let installing = $state(false);
  let showMore = $state(shot.onboardMore || import.meta.env.VITE_START_ONBOARD_MORE === "1");
  let lastRec = $state<Recommendation | null>(null);
  let installHeading = $state<HTMLHeadingElement | null>(null);

  $effect(() => {
    api
      .machineProfile()
      .then((m) => (machine = m))
      .catch(notifyInvokeError);
  });

  const download = $derived(
    Object.values(app.downloads).find(
      (d) => (d.kind === "model" || d.kind === "engine") && !d.done && !d.error,
    ),
  );
  const modelDone = $derived(!!app.settings?.activeModel);
  const engineReady = $derived(app.engine.state === "ready");
  const busy = $derived(!!download || installing);
  const failedDownload = $derived.by(() => {
    if (download || modelDone) return undefined;
    const preferredId = lastRec ? `model:${lastRec.reference}` : null;
    if (preferredId) {
      const preferred = app.downloads[preferredId];
      if (preferred?.error && preferred.error !== "cancelled") return preferred;
    }
    return Object.values(app.downloads).find(
      (d) => (d.kind === "model" || d.kind === "engine") && !!d.error && d.error !== "cancelled",
    );
  });
  const failedMessage = $derived(
    failedDownload?.error ? downloadErrorMessage(failedDownload.error) : null,
  );

  async function installRec(rec: Recommendation) {
    lastRec = rec;
    installing = true;
    try {
      await beginModelInstall("huggingface", rec.reference, rec.name, rec.license, rec.approxBytes);
    } catch (error) {
      installing = false;
      notifyInvokeError(error);
    }
  }

  async function skip() {
    const inFlight = Object.values(app.downloads).filter(
      (d) => (d.kind === "model" || d.kind === "engine") && !d.done && !d.error,
    );
    await Promise.all(inFlight.map((d) => api.downloadCancel(d.id).catch(() => undefined)));
    await finish();
  }

  async function finish() {
    try {
      await api.finishOnboarding();
      await refreshSettings();
      app.onboarding = false;
    } catch (error) {
      notifyInvokeError(error);
    }
  }

  $effect(() => {
    if (installing && Object.values(app.downloads).some((d) => d.kind === "model" && d.error)) {
      installing = false;
    }
  });

  $effect(() => {
    if (
      installing &&
      (app.engine.state === "error" || app.engine.state === "no-model") &&
      !download
    ) {
      installing = false;
    }
  });

  $effect(() => {
    if (installing && engineReady && !download) {
      finish();
    }
  });

  $effect(() => {
    if (step !== "model") return;
    void tick().then(() => installHeading?.focus());
  });
</script>

<div
  data-tauri-drag-region
  class="flex h-full items-start justify-center overflow-y-auto bg-navy-950 p-4 select-none min-[900px]:p-8"
>
  <div
    data-tauri-drag-region="false"
    class="onboard-stage my-auto w-full max-w-[600px] py-4 select-none"
  >
    {#if step === "promise"}
      <div
        class="onboard-pane flex flex-col items-center text-center"
        out:fade={{ duration: motionMs(200) }}
      >
        <img src={mark} alt="Larra" class="mb-5 w-[100px] rounded-2xl" />
        <h1 class="text-[28px] font-bold text-white">{t("onboarding.welcome")}</h1>
        <p class="mt-2 max-w-md text-[15px] leading-relaxed whitespace-pre-line text-white/65">
          {t("onboarding.lede")}
        </p>
        <div class="mt-8 grid w-full grid-cols-1 gap-3 min-[900px]:grid-cols-3">
          <div class="onboard-card rounded-xl bg-white/6 px-4 py-4 text-left">
            <MessageCircle size={17} class="mb-2 text-mint" />
            <p class="text-[14px] font-semibold text-white">{t("onboarding.cardChatTitle")}</p>
            <p class="mt-1 text-[13px] leading-snug text-white/55 min-[900px]:min-h-[4.5rem]">
              {t("onboarding.cardChatBody")}
            </p>
          </div>
          <div
            class="onboard-card rounded-xl bg-white/6 px-4 py-4 text-left"
            style="animation-delay: 60ms"
          >
            <LibraryBig size={17} class="mb-2 text-mint" />
            <p class="text-[14px] font-semibold text-white">{t("onboarding.cardShelfTitle")}</p>
            <p class="mt-1 text-[13px] leading-snug text-white/55 min-[900px]:min-h-[4.5rem]">
              {t("onboarding.cardShelfBody")}
            </p>
          </div>
          <div
            class="onboard-card rounded-xl bg-white/6 px-4 py-4 text-left"
            style="animation-delay: 120ms"
          >
            <ChefHat size={17} class="mb-2 text-mint" />
            <p class="text-[14px] font-semibold text-white">{t("onboarding.cardRecipesTitle")}</p>
            <p class="mt-1 text-[13px] leading-snug text-white/55 min-[900px]:min-h-[4.5rem]">
              {t("onboarding.cardRecipesBody")}
            </p>
          </div>
        </div>
        <button
          type="button"
          class="btn-amber mt-9 !px-7 !py-2.5 !text-[14px]"
          onclick={() => (step = "model")}
        >
          {t("onboarding.continue")}
        </button>
      </div>
    {:else}
      <section
        class="onboard-pane flex flex-col items-center rounded-3xl bg-white/[0.04] px-4 py-5 text-center ring-1 ring-white/15 min-[900px]:px-8 min-[900px]:py-8"
        aria-labelledby="onboard-install-heading"
        aria-busy={busy}
        in:installCard
      >
        {#snippet skipButton()}
          <button
            type="button"
            class="btn-ghost relative ml-auto shrink-0 py-2 pr-2 pl-3 !text-[0.8125rem] !text-white/55 hover:!bg-white/8 hover:!text-white"
            onclick={skip}
          >
            <span
              class="absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            ></span>
            {t("onboarding.skipLater")}
          </button>
        {/snippet}
        <h1
          id="onboard-install-heading"
          bind:this={installHeading}
          tabindex="-1"
          class="max-w-md text-[28px] font-bold text-balance text-white outline-none focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-white/40"
        >
          {t("onboarding.installHeading")}
        </h1>
        <p class="mt-2 max-w-md text-[15px] leading-relaxed text-white/65">
          {t("onboarding.installLede")}
        </p>

        {#if download}
          <div class="mt-8 flex w-full flex-col gap-3">
            <div class="onboard-card rounded-xl bg-white/10 px-4 py-4 text-left">
              <DownloadProgress
                {download}
                cancelable={download.phase !== "verifying"}
                onDark
                skipVerifyLabel={t("onboarding.skipVerify")}
                note={download.phase === "verifying"
                  ? t("onboarding.verifyingNote")
                  : t("onboarding.installingNote")}
              />
            </div>
            {#if download.phase !== "verifying"}
              <div class="flex justify-end">
                {@render skipButton()}
              </div>
            {/if}
          </div>
        {:else if modelDone}
          <p class="mt-8 flex items-baseline gap-2 text-[15px] font-medium text-mint">
            <Check size={16} class="size-4 h-lh shrink-0" aria-hidden="true" />
            {t("onboarding.installed")}
          </p>
          <button type="button" class="btn-amber mt-9 !px-7 !py-2.5 !text-[14px]" onclick={finish}>
            {t("onboarding.startUsing")}
          </button>
        {:else}
          <div class="mt-8 flex w-full flex-col gap-3">
            {#if machine}
              <p class="text-[15px] leading-relaxed text-white/55">
                {t("onboarding.severalFree")}
              </p>
              <div
                class="onboard-card flex flex-wrap items-start gap-4 rounded-xl bg-white/10 px-4 py-4 text-left"
              >
                <Download size={17} class="mt-0.5 shrink-0 text-mint" aria-hidden="true" />
                <div class="flex min-w-0 flex-1 basis-48 flex-col gap-1">
                  <p class="text-[12.5px] font-semibold text-white">
                    {failedDownload ? t("onboarding.couldntInstall") : t("onboarding.chosen")}
                  </p>
                  <p class="flex min-w-0 items-center gap-1 text-[15px] font-semibold text-white">
                    <span class="min-w-0 truncate">{(lastRec ?? machine.recommendation).name}</span>
                    <ModelCatalogInfo rec={lastRec ?? machine.recommendation} onDark />
                  </p>
                  <p class="text-[11.5px] leading-snug text-white/55 tabular-nums">
                    {t("onboarding.aboutDownload", {
                      size: formatBytes((lastRec ?? machine.recommendation).approxBytes),
                    })}
                  </p>
                  {#if failedMessage}
                    <p class="mt-1 text-[11.5px] leading-snug text-red-300" role="alert">
                      {failedMessage}
                    </p>
                  {/if}
                </div>
                {#if failedDownload}
                  <button
                    type="button"
                    class="btn-amber shrink-0"
                    onclick={() => machine && installRec(lastRec ?? machine.recommendation)}
                  >
                    {t("onboarding.tryAgain")}
                  </button>
                {:else if !busy}
                  <button
                    type="button"
                    class="btn-amber shrink-0"
                    onclick={() => machine && installRec(machine.recommendation)}
                  >
                    {t("onboarding.install")}
                  </button>
                {/if}
              </div>

              <div class="flex flex-wrap items-center gap-3">
                {#if machine.alternatives.length > 0}
                  <button
                    type="button"
                    class="btn-ghost relative py-2 pr-2 pl-3 !text-[0.8125rem] !text-white/55 hover:!bg-white/8 hover:!text-white"
                    onclick={() => (showMore = !showMore)}
                    aria-expanded={showMore}
                    aria-controls="onboard-other-ais"
                  >
                    <span
                      class="absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
                      aria-hidden="true"
                    ></span>
                    {t("onboarding.chooseDifferent")}
                    <ChevronDown
                      size={16}
                      class="size-4 h-lh shrink-0 {showMore ? 'rotate-180' : ''}"
                      aria-hidden="true"
                    />
                  </button>
                {/if}
                {@render skipButton()}
              </div>
              {#if showMore && machine.alternatives.length > 0}
                <ul
                  id="onboard-other-ais"
                  class="onboard-card overflow-hidden rounded-xl bg-white/10"
                  role="list"
                  transition:accordion
                >
                  {#each machine.alternatives as alt (alt.reference)}
                    <li
                      class="flex items-start gap-3 px-4 py-3 text-left not-last:border-b not-last:border-white/10"
                    >
                      <div class="min-w-0 flex-1">
                        <p
                          class="flex min-w-0 items-center gap-1 text-[12.5px] font-semibold text-white"
                        >
                          <span class="min-w-0 truncate">{alt.name}</span>
                          <ModelCatalogInfo rec={alt} onDark />
                        </p>
                        <p class="text-[11.5px] leading-snug text-white/55 tabular-nums">
                          {t("onboarding.aboutDownload", { size: formatBytes(alt.approxBytes) })}
                        </p>
                      </div>
                      <button
                        type="button"
                        class="btn-amber shrink-0 !px-4 !text-[12px]"
                        onclick={() => installRec(alt)}
                        disabled={busy}
                      >
                        {t("onboarding.install")}
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            {:else}
              <div class="flex justify-end">
                {@render skipButton()}
              </div>
            {/if}
          </div>
        {/if}
      </section>
    {/if}
  </div>
</div>

<style>
  .onboard-stage {
    display: grid;
    align-items: center;
    justify-items: stretch;
  }
  .onboard-pane {
    grid-area: 1 / 1;
  }
  @media (prefers-reduced-motion: no-preference) {
    .onboard-card {
      animation: onboard-card-in 280ms cubic-bezier(0.32, 0.72, 0, 1) both;
    }
  }
  @keyframes onboard-card-in {
    from {
      opacity: 0;
      transform: translateY(6px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
</style>
