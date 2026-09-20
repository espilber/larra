import { t } from "./i18n.svelte";
import type { ThinkLevel } from "./api";

export const THINK_LEVELS = ["off", "light", "deep"] as const satisfies readonly ThinkLevel[];

export function thinkLevelLabel(level: ThinkLevel): string {
  switch (level) {
    case "off":
      return t("shelves.thinkOff");
    case "light":
      return t("shelves.thinkLight");
    case "deep":
      return t("shelves.thinkDeep");
    default: {
      const _exhaustive: never = level;
      return _exhaustive;
    }
  }
}

export const THINK_LABELS: Record<ThinkLevel, string> = {
  get off() {
    return thinkLevelLabel("off");
  },
  get light() {
    return thinkLevelLabel("light");
  },
  get deep() {
    return thinkLevelLabel("deep");
  },
};

export function thinkLevelIndex(level: ThinkLevel): number {
  switch (level) {
    case "off":
      return 0;
    case "light":
      return 1;
    case "deep":
      return 2;
    default: {
      const _exhaustive: never = level;
      return _exhaustive;
    }
  }
}

export function thinkLevelFromIndex(index: number): ThinkLevel {
  const clamped = Math.max(0, Math.min(THINK_LEVELS.length - 1, Math.round(index)));
  return THINK_LEVELS[clamped] ?? "off";
}
