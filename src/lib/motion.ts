import { cubicOut } from "svelte/easing";
import { fade, fly, slide } from "svelte/transition";
import type { TransitionConfig } from "svelte/transition";

export function reducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export function motionMs(ms: number): number {
  return reducedMotion() ? 0 : ms;
}

export function overlay(node: Element): TransitionConfig {
  return fade(node, { duration: motionMs(150) });
}

export function drawerPanel(node: Element): TransitionConfig {
  if (reducedMotion()) return fade(node, { duration: 120 });
  return fly(node, { x: 24, duration: 280, opacity: 1, easing: cubicOut });
}

export function dialogPanel(node: Element): TransitionConfig {
  if (reducedMotion()) return fade(node, { duration: 120 });
  return fly(node, { y: 6, duration: 250, opacity: 1, easing: cubicOut });
}

export function sheetPanel(node: Element): TransitionConfig {
  if (reducedMotion()) return fade(node, { duration: 120 });
  return fly(node, { y: 8, duration: 250, opacity: 1, easing: cubicOut });
}

export function installCard(node: Element): TransitionConfig {
  if (reducedMotion()) return fade(node, { duration: 120 });
  return {
    duration: 280,
    easing: cubicOut,
    css: (t) => `opacity: ${t}; transform: translateY(${(1 - t) * 8}px) scale(${0.98 + 0.02 * t})`,
  };
}

export function accordion(node: Element): TransitionConfig {
  return slide(node, { duration: motionMs(200), easing: cubicOut });
}
