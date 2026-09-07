import { expect, it, vi } from "vitest";
import { boundedUpdateCheck } from "./updateSourceCheck";
it("bounds an unresponsive check and closes a late resource", async () => {
  vi.useFakeTimers();
  let resolve;
  const close = vi.fn(async () => {});
  const pending = boundedUpdateCheck(100, () => new Promise((done) => { resolve = done; }));
  const rejection = expect(pending).rejects.toThrow("timed out");
  await vi.advanceTimersByTimeAsync(100);
  await rejection;
  resolve({ close });
  await vi.advanceTimersByTimeAsync(0);
  expect(close).toHaveBeenCalledTimes(1);
  vi.useRealTimers();
});
