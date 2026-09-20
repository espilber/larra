/** Recipe prompts and the Chat composer. */
export const PROMPT_MAX_CHARS = 12_000;

/** House rules. Smaller: they go out with every message. */
export const HOUSE_RULES_MAX_CHARS = 4_000;

/** Saved thinking on a Chat message. */
export const THINKING_MAX_CHARS = 8_000;

export function clipChars(text: string, max: number): string {
  const chars = [...text];
  if (chars.length <= max) return text;
  return chars.slice(0, max).join("");
}
