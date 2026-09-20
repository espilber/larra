/** Debug screenshot runner: apply a job, then snapshot the parked window. */
import { tick } from "svelte";
import { api, events, invokeError, type ShotJob } from "./api";
import { isAppLocale } from "./i18n.svelte";
import { app, closeConversation, openThread, setUiLocale, type View } from "./stores.svelte";

export const shot = $state({
  source: false,
  thinking: false,
  docFirst: false,
  recipeNew: false,
  explore: false,
  onboard: "promise" as "promise" | "model",
  onboardMore: false,
  paneKey: 0,
});

function isView(value: string | undefined): value is View {
  return value === "chat" || value === "shelves" || value === "recipes" || value === "settings";
}

async function applyShot(job: ShotJob) {
  await api.closeAboutWindow().catch(() => undefined);

  shot.source = !!job.source;
  shot.thinking = !!job.thinking;
  shot.docFirst = job.doc === "first";
  shot.recipeNew = job.recipe === "new";
  shot.explore = !!job.explore;
  shot.onboard = job.onboard === "model" ? "model" : "promise";
  shot.onboardMore = !!job.onboardMore;

  if (job.locale && isAppLocale(job.locale)) {
    await setUiLocale(job.locale);
  }

  if (job.onboarding) {
    app.onboarding = true;
    shot.paneKey += 1;
    await tick();
    return;
  }

  app.onboarding = false;

  if (job.thread && job.thread >= 1 && app.threads.length > 0) {
    const thread = app.threads[job.thread - 1] ?? app.threads[0];
    if (thread) await openThread(thread.id);
  } else if (job.view === "chat" && !job.thread) {
    closeConversation();
  }

  if (isView(job.view)) {
    app.view = job.view;
    if (job.view === "shelves") {
      app.openShelfId = job.shelf === "first" ? (app.shelves[0]?.id ?? null) : null;
    }
  }

  shot.paneKey += 1;
  await tick();

  if (job.about) {
    await api.showAboutWindow();
  }
}

async function runShot(job: ShotJob) {
  try {
    await applyShot(job);
    await api.devSnapshot(
      job.path,
      job.label ?? (job.about ? "about" : "main"),
      job.settleMs ?? 700,
    );
    if (job.about) {
      await api.closeAboutWindow().catch(() => undefined);
    }
  } catch (error) {
    const message = invokeError(error);
    console.error(message);
    await api.devShotFail(message).catch(() => undefined);
  }
}

export function startShotControl() {
  void api.devShotReady();
  void events.shotJob((job) => {
    void runShot(job);
  });
}
