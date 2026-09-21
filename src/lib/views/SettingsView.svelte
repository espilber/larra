<script lang="ts">
  import {
    api,
    userFacingError,
    formatBytes,
    type Diagnostics,
    type MachineView,
    type ModelSearchResult,
    type Recommendation,
    type TextSize,
  } from "$lib/api";
  import {
    app,
    chatState,
    invalidateSettingsLoads,
    beginModelInstall,
    notifyInvokeError,
    refreshSettings,
    setTextSize,
    setUiLocale,
  } from "$lib/stores.svelte";
  import { isMac } from "$lib/platform";
  import { parseTextSize, textSizeLabel, TEXT_SIZES } from "$lib/text-size";
  import { shot } from "$lib/shot-control.svelte";
  import { HOUSE_RULES_MAX_CHARS, clipChars } from "$lib/text-cap";
  import {
    APP_LOCALES,
    parseAppLocale,
    parseLocalePref,
    t,
    type LocalePref,
  } from "$lib/i18n.svelte";
  import { Cpu, Search, BadgeCheck, Stethoscope, Save, Server, ChevronDown } from "@lucide/svelte";
  import DownloadProgress from "$lib/components/DownloadProgress.svelte";
  import ResetWorkspaceModal from "$lib/components/ResetWorkspaceModal.svelte";
  import ExploreModelsModal from "$lib/components/ExploreModelsModal.svelte";
  import ModelSuggestionCards from "$lib/components/ModelSuggestionCards.svelte";
  import icon from "../../assets/larra-icon.png";

  let machine = $state<MachineView | null>(null);
  let houseRules = $state(app.rulesDraft ?? app.settings?.houseRules ?? "");
  let rulesBaseline = $state(app.settings?.houseRules ?? "");
  let rulesError = $state("");
  let rulesSaving = $state(false);
  let onlineResearch = $state(app.settings?.allowOnlineResearch ?? false);
  let textSize = $state<TextSize>(parseTextSize(app.settings?.textSize));
  let uiLocale = $state<LocalePref>(parseLocalePref(app.settings?.uiLocale));
  const mac = isMac();
  let rulesSaved = $state(false);
  let diag = $state<Diagnostics | null>(null);
  let showDiag = $state(import.meta.env.VITE_START_DIAG === "1");
  let showReset = $state(false);
  let showExplore = $state(shot.explore || import.meta.env.VITE_START_EXPLORE === "1");
  let openedAbout = $state(false);
  let resetting = $state(false);
  let measuring = $state(false);
  let measureStatus = $state("");
  const chatBusy = $derived(
    Object.keys(chatState.pending).length > 0 || Object.keys(chatState.outbound).length > 0,
  );

  // --- External model endpoint (URL + key + detected models) ---
  const ENDPOINT_ERROR_KEYS = [
    "endpointBaseUrlMissing",
    "endpointUrlTooLong",
    "endpointUrlInvalid",
    "endpointSchemeUnsupported",
    "endpointUserInfoNotAllowed",
    "endpointModelIdMissing",
    "endpointModelIdTooLong",
    "endpointApiKeyTooLong",
  ] as const;

  function endpointError(error: unknown): string {
    const raw = typeof error === "string" ? error : String(error);
    const key = raw.trim();
    if (ENDPOINT_ERROR_KEYS.includes(key as (typeof ENDPOINT_ERROR_KEYS)[number])) {
      return t(`errors.${key}`);
    }
    return userFacingError(error);
  }

  let endpointOpen = $state(
    app.settings?.activeModel?.endpoint !== undefined && app.settings?.activeModel?.source === "external",
  );
  let endpointUrl = $state("");
  let endpointKey = $state("");
  let endpointBusy = $state(false);
  let endpointModels = $state<Array<{ id: string }>>([]);
  let endpointPicked = $state("");
  let endpointNote = $state("");

  const endpointLoopback = $derived(
    /^(http|https):\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/.test(endpointUrl.trim().toLowerCase()),
  );

  async function probeEndpoint() {
    if (endpointBusy) return;
    endpointBusy = true;
    endpointNote = "";
    endpointModels = [];
    endpointPicked = "";
    try {
      const models = await api.externalEndpointProbe(
        endpointUrl.trim(),
        endpointKey.trim() || undefined,
      );
      endpointModels = models;
      if (models.length === 0) {
        endpointNote = t("settings.externalNoModels");
      } else if (models.length === 1 && models[0]) {
        endpointPicked = models[0].id;
      }
    } catch (error) {
      endpointNote = endpointError(error);
    } finally {
      endpointBusy = false;
    }
  }

  async function useEndpointModel() {
    if (endpointBusy || !endpointPicked) return;
    endpointBusy = true;
    endpointNote = "";
    try {
      await api.externalModelSet(
        endpointUrl.trim(),
        endpointPicked,
        endpointKey.trim() || undefined,
      );
      await refreshSettings();
      endpointNote = t("settings.externalInUse");
    } catch (error) {
      endpointNote = endpointError(error);
    } finally {
      endpointBusy = false;
    }
  }

  async function stopUsingEndpoint() {
    endpointBusy = true;
    try {
      await api.externalModelClear();
      await refreshSettings();
      endpointNote = "";
    } catch (error) {
      endpointNote = endpointError(error);
    } finally {
      endpointBusy = false;
    }
  }

  async function measureAgain() {
    if (measuring) return;
    measuring = true;
    measureStatus = "";
    try {
      await api.engineRemeasure();
      diag = await api.diagnostics();
      measureStatus = t("settings.measureDone");
    } catch {
      measureStatus = t("settings.measureFailed");
    } finally {
      measuring = false;
    }
  }
  const hasAi = $derived(!!app.settings?.activeModel);

  $effect(() => {
    void app.settings?.activeModel?.reference;
    let cancelled = false;
    api
      .machineProfile()
      .then((m) => {
        if (!cancelled) machine = m;
      })
      .catch(notifyInvokeError);
    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const currentRules = clipChars(app.settings?.houseRules ?? "", HOUSE_RULES_MAX_CHARS);
    if (houseRules === rulesBaseline) houseRules = currentRules;
    rulesBaseline = currentRules;
    onlineResearch = app.settings?.allowOnlineResearch ?? false;
    textSize = parseTextSize(app.settings?.textSize);
    uiLocale = parseLocalePref(app.settings?.uiLocale);
  });

  $effect(() => {
    if (openedAbout) return;
    if (import.meta.env.VITE_START_ABOUT !== "1") return;
    openedAbout = true;
    api.showAboutWindow().catch(notifyInvokeError);
  });

  const modelDownload = $derived(
    Object.values(app.downloads).find((d) => d.kind === "model" && !d.done && !d.error),
  );
  const engineDownload = $derived(
    Object.values(app.downloads).find((d) => d.kind === "engine" && !d.done && !d.error),
  );

  function installFrom(result: ModelSearchResult) {
    void install(
      result.source.includes("huggingface") ? "huggingface" : "ollama",
      result.reference,
      result.name,
      result.license,
      result.sizeBytes,
    );
  }

  async function install(
    source: string,
    reference: string,
    name: string,
    license?: string,
    total?: number | null,
  ) {
    await beginModelInstall(source, reference, name, license, total);
  }

  function installRec(rec: Recommendation) {
    void install("huggingface", rec.reference, rec.name, rec.license, rec.approxBytes);
  }

  async function saveRules() {
    if (rulesSaving) return;
    const saved = clipChars(houseRules, HOUSE_RULES_MAX_CHARS);
    rulesSaving = true;
    rulesError = "";
    rulesSaved = false;
    try {
      invalidateSettingsLoads();
      await api.setHouseRules(saved);
      invalidateSettingsLoads();
      rulesBaseline = saved;
      if (app.settings) app.settings.houseRules = saved;
      rulesSaved = houseRules === saved;
      app.rulesDraft = rulesSaved ? null : houseRules;
    } catch (error) {
      rulesError = userFacingError(error);
    } finally {
      rulesSaving = false;
    }
  }

  function saveTextSize(next: TextSize) {
    textSize = next;
    void setTextSize(next);
  }

  function saveUiLocale(next: LocalePref) {
    uiLocale = next;
    void setUiLocale(next);
  }

  const localeHelp = $derived(
    uiLocale === "system"
      ? t("locale.helpUsing", {
          language: t(`locale.name_${parseAppLocale(app.settings?.resolvedLocale)}`),
        })
      : t("locale.help"),
  );

  const engineStatusLabel = $derived(
    app.engine.state === "ready"
      ? t("settings.engineReady")
      : app.engine.state === "starting"
        ? t("settings.engineWarming")
        : t("settings.engineIdle"),
  );

  async function saveOnline() {
    const next = onlineResearch;
    try {
      invalidateSettingsLoads();
      await api.setAllowOnlineResearch(next);
      invalidateSettingsLoads();
      await refreshSettings();
    } catch (error) {
      onlineResearch = app.settings?.allowOnlineResearch ?? false;
      notifyInvokeError(error);
    }
  }

  async function openDiagnostics() {
    showDiag = !showDiag;
    if (showDiag) diag = await api.diagnostics();
  }

  async function resetWorkspace() {
    resetting = true;
    try {
      await api.resetWorkspace("DELETE");
    } catch (error) {
      resetting = false;
      notifyInvokeError(error);
    }
  }
</script>

<div class="mx-auto max-w-[760px] px-4 py-5 min-[900px]:px-8 min-[900px]:py-8">
  <h1 class="mb-6 text-[22px] font-semibold text-ink">{t("settings.title")}</h1>

  <section class="card mb-6 px-4 py-5 min-[900px]:px-6">
    <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-3">
      <div class="min-w-0 flex-1 basis-64">
        <h2 id="ui-locale-heading" class="text-[15px] font-semibold text-ink">
          {t("locale.heading")}
        </h2>
        <p id="ui-locale-help" class="mt-0.5 text-[12.5px] leading-snug text-ink-soft">
          {localeHelp}
        </p>
      </div>
      <label class="sr-only" for="ui-locale">{t("locale.heading")}</label>
      <span class="select-wrap w-auto min-w-[12rem] shrink-0">
        <select
          id="ui-locale"
          name="ui-locale"
          class="input select h-9 cursor-default py-0"
          aria-labelledby="ui-locale-heading"
          aria-describedby="ui-locale-help"
          bind:value={uiLocale}
          onchange={() => saveUiLocale(uiLocale)}
        >
          <option value="system">{t("locale.system")}</option>
          {#each APP_LOCALES as code (code)}
            <option value={code}>{t(`locale.${code}`)}</option>
          {/each}
        </select>
        <svg
          viewBox="0 0 8 5"
          width="8"
          height="5"
          fill="none"
          class="pointer-events-none col-start-2 row-start-1 place-self-center text-ink-faint"
          aria-hidden="true"
        >
          <path d="M.5.5 4 4 7.5.5" stroke="currentcolor" stroke-linecap="round" />
        </svg>
      </span>
    </div>
  </section>

  <section class="card mb-6 px-4 py-5 min-[900px]:px-6">
    <div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-3">
      <div class="min-w-0 flex-1 basis-64">
        <h2 id="text-size-heading" class="text-[15px] font-semibold text-ink">
          {t("settings.textSize")}
        </h2>
        <p id="text-size-help" class="mt-0.5 text-[12.5px] leading-snug text-ink-soft">
          {t("settings.textSizeHelp")}
          {mac ? t("settings.textSizeKeysMac") : t("settings.textSizeKeysWin")}
        </p>
      </div>
      <div
        role="radiogroup"
        aria-labelledby="text-size-heading"
        aria-describedby="text-size-help"
        class="segmented"
      >
        {#each TEXT_SIZES as size (size)}
          <label
            class="segmented-item {size === 'default'
              ? 'text-[12px]'
              : size === 'large'
                ? 'text-[13px]'
                : 'text-[14px]'}"
          >
            <span
              class="absolute top-1/2 left-1/2 size-[max(100%,3rem)] -translate-1/2 pointer-fine:hidden"
              aria-hidden="true"
            ></span>
            <input
              id="text-size-{size}"
              name="text-size"
              type="radio"
              value={size}
              class="sr-only"
              checked={textSize === size}
              onchange={() => saveTextSize(size)}
            />
            {textSizeLabel(size)}
          </label>
        {/each}
      </div>
    </div>
  </section>

  {#if hasAi}
    {@render houseRulesSection()}
    {@render onlineSection()}
    {@render aiSection()}
  {:else}
    {@render aiSection()}
    {@render houseRulesSection()}
    {@render onlineSection()}
  {/if}

  {#snippet houseRulesSection()}
    <section class="card mb-6 px-4 py-5 min-[900px]:px-6">
      <h2 class="mb-1 text-[15px] font-semibold text-ink">{t("settings.houseRules")}</h2>
      <p class="mb-3 text-[12.5px] leading-snug text-ink-soft">
        {t("settings.houseRulesHelp")}
      </p>
      <label class="sr-only" for="house-rules">{t("settings.houseRules")}</label>
      <textarea
        id="house-rules"
        name="house-rules"
        class="input min-h-32 cursor-text resize-y select-text"
        placeholder={t("settings.houseRulesPlaceholder")}
        maxlength={HOUSE_RULES_MAX_CHARS}
        bind:value={houseRules}
        oninput={(event) => {
          app.rulesDraft = event.currentTarget.value;
          rulesSaved = false;
        }}
        aria-describedby={rulesError ? "rules-error" : undefined}></textarea>
      {#if rulesError}<p
          id="rules-error"
          role="alert"
          class="mt-2 text-sm text-red-700 dark:text-red-400"
        >
          {rulesError}
        </p>{/if}
      <div class="mt-2.5 flex justify-end">
        <button type="button" class="btn-primary" onclick={saveRules} disabled={rulesSaving}>
          <Save size={13.5} />
          {rulesSaved ? t("settings.saved") : t("settings.saveHouseRules")}
        </button>
      </div>
    </section>
  {/snippet}

  {#snippet onlineSection()}
    <section class="card mb-6 px-4 py-5 min-[900px]:px-6">
      <label class="flex cursor-default items-start gap-3" for="online-research">
        <input
          id="online-research"
          name="online-research"
          type="checkbox"
          aria-describedby="online-research-help"
          class="mt-1.5 size-4 shrink-0 rounded border-paper-line accent-navy-500 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-800"
          bind:checked={onlineResearch}
          onchange={() => void saveOnline()}
        />
        <span>
          <h2 class="text-[15px] font-semibold text-ink">{t("settings.online")}</h2>
          <p id="online-research-help" class="mt-1 text-[12.5px] leading-snug text-ink-soft">
            {t("settings.onlineHelp")}
          </p>
        </span>
      </label>
    </section>
  {/snippet}

  {#snippet aiSection()}
    <section class="card mb-6 px-4 py-5 min-[900px]:px-6">
      <h2 class="mb-1 text-[15px] font-semibold text-ink">{t("settings.ai")}</h2>
      {#if machine}
        <p class="mb-4 flex flex-wrap items-center gap-1.5 text-[12px] text-ink-faint">
          <Cpu size={12.5} />
          {machine.profile.cpu} · {formatBytes(machine.profile.totalRamBytes)} memory ·
          {formatBytes(machine.profile.freeDiskBytes)} free disk
        </p>
      {/if}

      {#if modelDownload}
        <div class="mb-4 rounded-xl border border-paper-line bg-paper-soft px-4 py-3">
          <DownloadProgress
            download={modelDownload}
            cancelable
            note={app.settings?.activeModel ? t("settings.keepUsingChat") : undefined}
          />
        </div>
      {:else if engineDownload}
        <div class="mb-4 rounded-xl border border-paper-line bg-paper-soft px-4 py-3">
          <DownloadProgress download={engineDownload} cancelable />
        </div>
      {/if}

      {#if app.settings?.activeModel}
        {@const model = app.settings.activeModel}
        <div
          class="flex flex-wrap items-center gap-3 rounded-xl border border-paper-line bg-paper-soft/50 px-4 py-3"
        >
          <BadgeCheck size={18} class="shrink-0 text-navy-600 dark:text-navy-400" />
          <div class="min-w-0 flex-1">
            <p class="text-[13.5px] font-semibold text-ink">{model.name}</p>
            {#if model.endpoint}
              <p class="text-[11.5px] text-ink-soft">
                {t("settings.externalSource")} · {model.endpoint.baseUrl}
                {#if !/^(http|https):\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/.test(model.endpoint.baseUrl.toLowerCase())}
                  · {t("settings.externalRemoteNote")}
                {/if}
              </p>
            {:else}
              <p class="text-[11.5px] text-ink-soft">
                {formatBytes(model.sizeBytes)}{model.license ? ` · ${model.license}` : ""} ·
                {t("settings.installedHere")}
              </p>
            {/if}
          </div>
          <span
            class="rounded-full px-2 py-1 text-[10.5px] font-semibold
          {app.engine.state === 'ready'
              ? 'bg-ready text-ready-ink dark:bg-navy-200/20 dark:text-navy-200'
              : app.engine.state === 'starting'
                ? 'bg-amber-350/50 text-amber-550'
                : 'bg-paper-soft text-ink-faint'}"
          >
            {engineStatusLabel}
          </span>
          {#if model.endpoint}
            <button
              type="button"
              class="btn-outline shrink-0"
              onclick={stopUsingEndpoint}
              disabled={endpointBusy}
            >
              {t("settings.externalStopUsing")}
            </button>
          {/if}
        </div>
      {/if}

      <div class="mt-5 border-t border-navy-950/10 pt-5 dark:border-white/10">
        <button
          type="button"
          class="flex w-full flex-wrap items-center gap-3 text-left"
          aria-expanded={endpointOpen}
          onclick={() => (endpointOpen = !endpointOpen)}
        >
          <Server size={16} class="shrink-0 text-navy-600 dark:text-navy-400" />
          <span class="min-w-0 flex-1">
            <span class="block text-[15px] font-semibold text-ink">
              {t("settings.externalEndpoint")}
            </span>
            <span class="mt-0.5 block text-[12.5px] leading-snug text-ink-soft">
              {t("settings.externalEndpointLede")}
            </span>
          </span>
          <ChevronDown
            size={15}
            class="shrink-0 transition-transform {endpointOpen ? 'rotate-180' : ''}"
          />
        </button>
        {#if endpointOpen}
          <div class="mt-4 rounded-xl border border-paper-line bg-paper-soft/50 px-4 py-3">
            <label class="block text-[12px] font-medium text-ink" for="endpoint-url">
              {t("settings.externalUrl")}
            </label>
            <input
              id="endpoint-url"
              type="text"
              class="mt-1 w-full rounded-lg border border-paper-line bg-paper px-3 py-2 text-[13px] text-ink"
              placeholder={t("settings.externalUrlPlaceholder")}
              bind:value={endpointUrl}
            />
            <label class="mt-3 block text-[12px] font-medium text-ink" for="endpoint-key">
              {t("settings.externalApiKey")}
            </label>
            <input
              id="endpoint-key"
              type="password"
              class="mt-1 w-full rounded-lg border border-paper-line bg-paper px-3 py-2 text-[13px] text-ink"
              placeholder="—"
              bind:value={endpointKey}
            />
            {#if endpointUrl.trim() && !endpointLoopback}
              <p class="mt-2 text-[11.5px] text-amber-550">{t("settings.externalRemoteWarn")}</p>
            {/if}
            <div class="mt-3 flex flex-wrap items-center gap-3">
              <button
                type="button"
                class="btn-outline"
                onclick={probeEndpoint}
                disabled={endpointBusy || !endpointUrl.trim()}
              >
                {endpointBusy ? t("settings.externalDetecting") : t("settings.externalDetect")}
              </button>
              {#if endpointModels.length > 0}
                <select
                  class="min-w-0 flex-1 rounded-lg border border-paper-line bg-paper px-3 py-2 text-[13px] text-ink"
                  bind:value={endpointPicked}
                  aria-label={t("settings.externalPick")}
                >
                  {#each endpointModels as m (m.id)}
                    <option value={m.id}>{m.id}</option>
                  {/each}
                </select>
                <button
                  type="button"
                  class="btn-outline shrink-0"
                  onclick={useEndpointModel}
                  disabled={endpointBusy || !endpointPicked}
                >
                  {t("settings.externalUseModel")}
                </button>
              {/if}
            </div>
            {#if endpointNote}
              <p class="mt-2 text-[12px] text-ink-soft">{endpointNote}</p>
            {/if}
          </div>
        {/if}
      </div>

      {#if machine && machine.suggestions.length > 0}
        <div class="mt-5">
          <ModelSuggestionCards
            suggestions={machine.suggestions}
            installing={!!modelDownload}
            heading={app.settings?.activeModel ? t("settings.otherAis") : t("settings.suggested")}
            lede={app.settings?.activeModel
              ? t("settings.otherAisLede")
              : t("settings.suggestedLede")}
            onInstall={installRec}
          />
        </div>
      {/if}

      <div
        class="mt-5 flex flex-wrap items-center gap-4 border-t border-navy-950/10 pt-5 dark:border-white/10"
      >
        <div class="min-w-0 flex-1">
          <h3 class="text-[15px] font-semibold text-ink">{t("settings.exploreHeading")}</h3>
          <p class="mt-0.5 text-[12.5px] leading-snug text-ink-soft">
            {t("settings.exploreLede")}
          </p>
        </div>
        <button
          type="button"
          class="btn-outline shrink-0"
          aria-haspopup="dialog"
          aria-expanded={showExplore}
          aria-controls={showExplore ? "explore-models-dialog" : undefined}
          onclick={() => (showExplore = true)}
        >
          <Search size={13.5} aria-hidden="true" />
          {t("settings.browseAis")}
        </button>
      </div>
    </section>
  {/snippet}

  <section class="card mb-6 px-4 py-5 min-[900px]:px-6">
    <div class="flex flex-wrap items-center gap-4">
      <img src={icon} alt="" class="h-12 w-12 rounded-[22%]" />
      <div class="min-w-0 flex-1">
        <h2 class="text-[15px] font-semibold text-ink">{t("settings.aboutHeading")}</h2>
        <p class="mt-0.5 text-[12.5px] leading-snug text-ink-soft">
          {t("officialLineShort")}
        </p>
      </div>
      <button
        type="button"
        class="btn-outline shrink-0"
        onclick={() => api.showAboutWindow().catch(notifyInvokeError)}
      >
        {t("settings.open")}
      </button>
    </div>
  </section>

  <section class="card mb-6 px-4 py-5 min-[900px]:px-6">
    <div class="flex flex-wrap items-center gap-4">
      <div class="min-w-0 flex-1">
        <h2 class="text-[15px] font-semibold text-ink">{t("settings.resetHeading")}</h2>
        <p class="mt-0.5 text-[12.5px] leading-snug text-ink-soft">
          {t("settings.resetHelp")}
        </p>
      </div>
      <button
        type="button"
        class="btn-outline shrink-0 text-red-700 hover:border-red-300 hover:bg-red-50 dark:text-red-400 dark:hover:border-red-400/40 dark:hover:bg-red-400/10"
        aria-haspopup="dialog"
        aria-expanded={showReset}
        aria-controls={showReset ? "reset-workspace-dialog" : undefined}
        onclick={() => (showReset = true)}
      >
        {t("settings.reset")}
      </button>
    </div>
  </section>

  <section class="mb-10">
    <button type="button" class="btn-ghost !text-[12px]" onclick={openDiagnostics}>
      <Stethoscope size={13} />
      {showDiag ? t("settings.hideDiagnostics") : t("settings.diagnostics")}
    </button>
    {#if showDiag && hasAi}
      <div class="mt-2 flex flex-wrap items-center gap-3">
        <button
          type="button"
          class="btn-outline"
          onclick={measureAgain}
          disabled={measuring || chatBusy || !!modelDownload || !!engineDownload}
        >
          {measuring ? t("settings.measuring") : t("settings.measureAgain")}
        </button>
        <p role="status" class="text-sm text-ink-soft">{measureStatus}</p>
      </div>
    {/if}
    {#if showDiag && diag}
      <div
        class="card mt-2 px-4 py-4 font-mono text-[11.5px] leading-relaxed break-words text-ink-soft"
      >
        <p>
          Larra {diag.version} · engine {diag.engineBuild} ({diag.engineState.state}{diag
            .engineState.detail
            ? ` · ${diag.engineState.detail}`
            : ""})
        </p>
        <p>data: {diag.dataDir}</p>
        <p class="mt-1 text-[10.5px] text-ink-faint">
          {t("settings.dataFolderHint")}
        </p>
        <p>
          model: {diag.model
            ? `${diag.model.name} · ${diag.model.file}`
            : t("settings.noneInstalled")}
        </p>
        <p>index records: {diag.indexRecords} · context budget: {diag.contextBudgetChars} chars</p>
        {#if diag.benchmark}
          <p>
            benchmark: {diag.benchmark.promptTokensPerSecond.toFixed(0)} pp tok/s · {diag.benchmark.generationTokensPerSecond.toFixed(
              0,
            )} gen tok/s · {diag.benchmark.measuredAt}
          </p>
        {/if}
        <p>
          machine: {diag.machine.cpu} · {formatBytes(diag.machine.totalRamBytes)} RAM · {diag
            .machine.accelerator}
          {diag.machine.processArch !== diag.machine.osArch
            ? ` · ${diag.machine.processArch} on ${diag.machine.osArch}`
            : ` · ${diag.machine.processArch}`}
        </p>
        <p>formats: {diag.supportedFormats.join(" ")}</p>
        {#if diag.engineLogPresent}
          <p class="mt-2">
            engine log location:
            <button
              type="button"
              class="rounded-sm text-left break-all underline decoration-navy-200 underline-offset-2 hover:text-ink hover:decoration-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-navy-800"
              aria-label={t("settings.openEngineLog")}
              onclick={() => api.openEngineLog().catch(notifyInvokeError)}
            >
              <code>{diag.engineLogPath}</code>
            </button>
          </p>
          <p class="mt-1 text-[10.5px] leading-relaxed text-ink-faint">
            {t("settings.logOnDisk")}
          </p>
        {/if}
      </div>
    {/if}
  </section>
</div>

{#if showExplore}
  <ExploreModelsModal
    installing={!!modelDownload}
    onClose={() => (showExplore = false)}
    onInstall={(result) => {
      installFrom(result);
      showExplore = false;
    }}
  />
{/if}
{#if showReset}
  <ResetWorkspaceModal
    busy={resetting}
    onClose={() => {
      if (!resetting) showReset = false;
    }}
    onConfirm={resetWorkspace}
  />
{/if}
