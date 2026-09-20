import { describe, expect, it } from "vitest";
import {
  PREFERRED_SHELF_KEY,
  loadPreferredShelf,
  savePreferredShelf,
  shelfForNewConversation,
} from "./shelf-preference";

describe("shelfForNewConversation", () => {
  it("uses the only Shelf when nothing has been chosen yet", () => {
    expect(shelfForNewConversation(undefined, ["s1"])).toBe("s1");
    expect(shelfForNewConversation(undefined, [])).toBeNull();
    expect(shelfForNewConversation(undefined, ["s1", "s2"])).toBeNull();
  });

  it("keeps an explicit No Shelf", () => {
    expect(shelfForNewConversation(null, ["s1"])).toBeNull();
    expect(shelfForNewConversation(null, ["s1", "s2"])).toBeNull();
  });

  it("keeps a manual Shelf while it still exists", () => {
    expect(shelfForNewConversation("s2", ["s1", "s2"])).toBe("s2");
    expect(shelfForNewConversation("gone", ["s1"])).toBeNull();
  });
});

describe("preferred shelf storage", () => {
  it("round-trips No Shelf and a Shelf id", () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
    };
    expect(loadPreferredShelf(storage)).toBeUndefined();
    savePreferredShelf(null, storage);
    expect(store.get(PREFERRED_SHELF_KEY)).toBe("");
    expect(loadPreferredShelf(storage)).toBeNull();
    savePreferredShelf("s1", storage);
    expect(loadPreferredShelf(storage)).toBe("s1");
  });
});
