import { describe, expect, it, vi } from "vitest";
import { createStore } from "./index";

describe("createStore", () => {
  it("値を更新すると購読者に通知する", () => {
    const store = createStore(0);
    const listener = vi.fn();
    store.subscribe(listener);
    store.set(1);
    expect(store.get()).toBe(1);
    expect(listener).toHaveBeenCalledWith(1);
  });

  it("購読を解除すると通知されない", () => {
    const store = createStore(0);
    const listener = vi.fn();
    store.subscribe(listener)();
    store.set(1);
    expect(listener).not.toHaveBeenCalled();
  });
});
