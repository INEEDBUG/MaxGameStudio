import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, test, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import CsWorkspacePage from "./CsWorkspacePage";

vi.mock("../i18n/useT.js", () => ({
  useT: () => (key, params) => params ? `${key}:${params.count}` : key,
}));

const routes = [
  ["csWorkspace.analysis.library.title", "/cs2/library"],
  ["csWorkspace.analysis.officialDemos.title", "/cs2/match-history"],
  ["csWorkspace.analysis.preview.title", "/cs2/analysis"],
  ["csWorkspace.production.queue.title", "/cs2/queue"],
  ["csWorkspace.production.assets.title", "/cs2/recorded-videos"],
  ["csWorkspace.production.montage.title", "/cs2/montage"],
  ["csWorkspace.production.liteCut.title", "/cs2/lite-cut"],
  ["csWorkspace.tools.recordingParams.title", "/cs2/params"],
  ["csWorkspace.tools.playerConfig.title", "/cs2/player-game-config"],
  ["csWorkspace.tools.guide.title", "/cs2/guide"],
];

function renderPage(props = {}) {
  return render(<MemoryRouter><CsWorkspacePage {...props} /></MemoryRouter>);
}

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("CsWorkspacePage", () => {
  test("shows the workspace title, three groups, and explicit zero queue count", () => {
    renderPage({ queueLength: 0 });

    expect(screen.getByRole("heading", { level: 1, name: "csWorkspace.title" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "csWorkspace.analysis.title" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "csWorkspace.production.title" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "csWorkspace.tools.title" })).toBeTruthy();
    expect(screen.getByText("0")).toBeTruthy();
    expect(screen.getByRole("link", { name: "csWorkspace.queueCount:0" })).toBeTruthy();
  });

  test("links to the existing CS2 routes", () => {
    renderPage();
    const links = screen.getAllByRole("link");
    const hrefs = links.map((link) => link.getAttribute("href"));

    for (const [, path] of routes) expect(hrefs).toContain(path);
    expect(hrefs).not.toContain('/cs2/cosmetic-studio');
  });

  test("disabled state blocks production and config destinations but preserves analysis navigation", () => {
    renderPage({ disabled: true });

    const disabledRoutes = routes.filter(([, path]) => ![
      "/cs2/library", "/cs2/match-history", "/cs2/analysis", "/cs2/guide",
    ].includes(path));
    const links = screen.getAllByRole("link");
    const hrefs = links.map((link) => link.getAttribute("href"));
    for (const [, path] of disabledRoutes) expect(hrefs).not.toContain(path);
    for (const path of ["/cs2/library", "/cs2/match-history", "/cs2/analysis", "/cs2/guide"])
      expect(hrefs).toContain(path);

    for (const [key] of disabledRoutes) {
      const label = screen.getByText(key);
      expect(label.closest('[aria-disabled="true"]')).toBeTruthy();
      expect(label.closest("a")).toBeNull();
    }
  });

  test("uses keyboard-focusable anchors and non-focusable disabled semantics", () => {
    renderPage({ disabled: true });
    const analysisLink = screen.getByRole("link", { name: "csWorkspace.analysis.preview.title" });
    analysisLink.focus();
    expect(document.activeElement).toBe(analysisLink);

    const disabled = screen.getByText("csWorkspace.production.montage.title").closest('[aria-disabled="true"]');
    expect(disabled).toBeTruthy();
    expect(disabled.closest("a")).toBeNull();
    expect(disabled.tabIndex).toBe(-1);
  });

  test("does not fetch, poll, or create side effects on mount", () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
    const intervalSpy = vi.spyOn(window, "setInterval");

    renderPage({ queueLength: 3 });

    expect(fetchMock).not.toHaveBeenCalled();
    expect(intervalSpy).not.toHaveBeenCalled();
    expect(document.querySelector("canvas")).toBeNull();
  });
});
