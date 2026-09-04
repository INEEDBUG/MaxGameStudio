import { describe, expect, test } from "vitest";
import {
  desktopCloseActionPayload,
  normalizeDesktopCloseAction,
} from "./desktopCloseAction";

describe("desktop close action contract", () => {
  test("normalizes supported actions and legacy values", () => {
    expect(normalizeDesktopCloseAction("ask")).toBe("ask");
    expect(normalizeDesktopCloseAction("tray")).toBe("tray");
    expect(normalizeDesktopCloseAction("exit")).toBe("exit");
    expect(normalizeDesktopCloseAction("unknown")).toBe("ask");
    expect(normalizeDesktopCloseAction(undefined, false)).toBe("exit");
  });

  test("keeps runtime and persisted close settings aligned", () => {
    expect(desktopCloseActionPayload("ask")).toEqual({ close_action: "ask", close_to_tray: true });
    expect(desktopCloseActionPayload("tray")).toEqual({ close_action: "tray", close_to_tray: true });
    expect(desktopCloseActionPayload("exit")).toEqual({ close_action: "exit", close_to_tray: false });
  });
});
