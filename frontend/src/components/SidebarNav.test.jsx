import { describe, expect, test, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import SidebarNav from "./SidebarNav";

globalThis.__APP_VERSION__ = "test";

vi.mock("../i18n/useT.js", () => ({ useT: () => (key) => key }));
vi.mock("../stores/themeStore", () => ({
  useThemeStore: (selector) => selector({
    mode: "system",
    resolvedTheme: "dark",
    setMode: vi.fn(),
  }),
}));
vi.mock("../stores/replayStore", () => ({
  useReplayStore: { getState: () => ({ requestSuspendPlayback: vi.fn() }) },
}));

describe("SidebarNav", () => {
  test("keeps the official demo downloader reachable after onboarding", () => {
    render(
      <MemoryRouter>
        <SidebarNav />
      </MemoryRouter>,
    );

    expect(screen.getByRole("link", { name: "nav.officialDemos" }).getAttribute("href")).toBe(
      "/match-history",
    );
    expect(screen.getByRole("link", { name: "nav.recordedVideos" }).getAttribute("href")).toBe(
      "/recorded-videos",
    );
  });

  test("keeps the appearance popover above the main workspace layer", () => {
    const { container } = render(
      <MemoryRouter>
        <SidebarNav />
      </MemoryRouter>,
    );

    expect(container.querySelector("aside")?.className).toContain("z-[60]");
    fireEvent.click(screen.getByRole("button", { name: "nav.themeSystem" }));
    const menu = screen.getByRole("menu", { name: "nav.appearance" });
    expect(menu).toBeTruthy();
    expect(menu.className).not.toContain("backdrop-blur");
  });
});
