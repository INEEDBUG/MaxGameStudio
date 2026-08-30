import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import SensitivityAimArena from "./SensitivityAimArena.jsx";

function canvasContext() {
  const gradient = { addColorStop: vi.fn() };
  return {
    setTransform: vi.fn(), clearRect: vi.fn(), createRadialGradient: vi.fn(() => gradient),
    fillRect: vi.fn(), beginPath: vi.fn(), moveTo: vi.fn(), lineTo: vi.fn(), stroke: vi.fn(),
    arc: vi.fn(), fill: vi.fn(), fillText: vi.fn(),
  };
}

describe("SensitivityAimArena click protocol", () => {
  let rafCallback;
  let currentNow;

  beforeEach(() => {
    rafCallback = null;
    currentNow = 0;
    vi.spyOn(performance, "now").mockImplementation(() => currentNow);
    vi.spyOn(window, "requestAnimationFrame").mockImplementation((callback) => {
      rafCallback = callback;
      return 1;
    });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
    vi.spyOn(Math, "random").mockReturnValue(0);
    vi.spyOn(HTMLCanvasElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 500, height: 400, left: 0, top: 0, right: 500, bottom: 400,
    });
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(canvasContext());
    Object.defineProperty(document, "pointerLockElement", { configurable: true, value: null, writable: true });
    document.exitPointerLock = vi.fn(() => {
      document.pointerLockElement = null;
      document.dispatchEvent(new Event("pointerlockchange"));
    });
    HTMLCanvasElement.prototype.requestPointerLock = vi.fn(function requestPointerLock() {
      document.pointerLockElement = this;
      document.dispatchEvent(new Event("pointerlockchange"));
      return Promise.resolve();
    });
  });

  afterEach(() => vi.restoreAllMocks());

  it("does not count cursor contact until the user clicks", () => {
    const onComplete = vi.fn();
    render(
      <SensitivityAimArena
        trial={{ kind: "flick", multiplier: 1 }}
        setup={{ current_sensitivity: 1, dpi: 800, m_yaw: 0.022 }}
        index={0}
        total={1}
        durationMs={3_000}
        onComplete={onComplete}
        onCancel={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /锁定鼠标并开始|Lock pointer & start/ }));
    currentNow = 4_000;
    act(() => rafCallback?.(currentNow));

    const move = new MouseEvent("mousemove", { bubbles: true });
    Object.defineProperties(move, {
      movementX: { value: -205 },
      movementY: { value: -155 },
    });
    act(() => document.dispatchEvent(move));
    expect(onComplete).not.toHaveBeenCalled();

    act(() => document.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button: 0 })));
    currentNow = 7_001;
    act(() => rafCallback?.(currentNow));

    expect(onComplete).toHaveBeenCalledOnce();
    expect(onComplete.mock.calls[0][0]).toMatchObject({
      clicks: 1,
      hits: 1,
      misses: 0,
      underflicks: 0,
      off_axis_misses: 0,
    });
  });
});
