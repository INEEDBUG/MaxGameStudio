import { beforeEach, describe, expect, test, vi } from "vitest";
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
  beforeEach(() => {
    window.localStorage.clear();
  });

  test("keeps the official demo downloader reachable after onboarding", () => {
    render(
      <MemoryRouter>
        <SidebarNav />
      </MemoryRouter>,
    );

    expect(screen.getByRole("link", { name: "nav.officialDemos" }).getAttribute("href")).toBe(
      "/cs2/match-history",
    );
    expect(screen.getByRole("link", { name: "nav.recordedVideos" }).getAttribute("href")).toBe(
      "/cs2/recorded-videos",
    );
  });

  test("exposes Home as an independent top-level route", () => {
    render(<MemoryRouter initialEntries={["/"]}><SidebarNav /></MemoryRouter>);
    expect(screen.getByRole("link", { name: "nav.home" }).getAttribute("href")).toBe("/");
    expect(screen.getByRole("link", { name: "nav.home" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("button", { name: "nav.sectionCs2" }).getAttribute("aria-current")).toBeNull();
  });

  test("groups game features and automatically opens the active section", () => {
    render(
      <MemoryRouter initialEntries={["/league/history"]}>
        <SidebarNav />
      </MemoryRouter>,
    );

    const leagueGroup = screen.getByRole("button", { name: "nav.sectionLeague" });
    expect(leagueGroup.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("link", { name: "nav.leagueHistory" }).getAttribute("href")).toBe("/league/history");
    expect(screen.queryByRole("link", { name: "nav.sensitivityLab" })).toBeNull();
  });

  test("allows the active section to stay collapsed until navigation changes", () => {
    render(
      <MemoryRouter initialEntries={["/league/history"]}>
        <SidebarNav />
      </MemoryRouter>,
    );
    const leagueGroup = screen.getByRole("button", { name: "nav.sectionLeague" });
    fireEvent.click(leagueGroup);
    expect(leagueGroup.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("link", { name: "nav.leagueHistory" })).toBeNull();
  });

  test("persists collapsible group state and exposes keyboard-friendly aria state", () => {
    render(
      <MemoryRouter initialEntries={["/settings"]}>
        <SidebarNav />
      </MemoryRouter>,
    );

    const valorantGroup = screen.getByRole("button", { name: "nav.sectionValorant" });
    expect(valorantGroup.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(valorantGroup);
    expect(valorantGroup.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByRole("link", { name: "nav.valorantStretch" }).getAttribute("href")).toBe("/valorant/stretch");
    expect(JSON.parse(window.localStorage.getItem("maxgamestudio.sidebar.groups.v1"))).toMatchObject({ valorant: true });
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
