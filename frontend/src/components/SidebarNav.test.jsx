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

  test("groups CS tools and exposes a lightweight workspace landing", () => {
    const { container } = render(<MemoryRouter initialEntries={["/cs2"]}><SidebarNav /></MemoryRouter>);
    expect(screen.getByRole("link", { name: "csWorkspace.title" }).getAttribute("href")).toBe("/cs2");
    for (const section of ["analysis", "production", "tools"]) {
      expect(screen.getByRole("group", { name: `csWorkspace.${section}.title` })).toBeTruthy();
    }
    expect(container.querySelector("aside").className).not.toContain("backdrop-blur");
  });

  test("disabled recording links cannot be activated with keyboard clicks", () => {
    render(<MemoryRouter initialEntries={["/cs2"]}><SidebarNav disabled /></MemoryRouter>);
    const queue = screen.getByRole("link", { name: "nav.recordQueue 0" });
    expect(queue.getAttribute("aria-disabled")).toBe("true");
    expect(queue.tabIndex).toBe(-1);
    fireEvent.click(queue);
    expect(queue.getAttribute("aria-current")).toBeNull();
    expect(screen.getByRole("link", { name: "csWorkspace.title" }).getAttribute("aria-current")).toBe("page");
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

  test("exposes the single League workspace as a direct route", () => {
    render(
      <MemoryRouter initialEntries={["/league/history"]}>
        <SidebarNav />
      </MemoryRouter>,
    );

    const leagueLink = screen.getByRole("link", { name: "nav.sectionLeague" });
    expect(leagueLink.getAttribute("href")).toBe("/league");
    expect(leagueLink.getAttribute("aria-current")).toBe("page");
    expect(screen.queryByRole("link", { name: "nav.sensitivityLab" })).toBeNull();
  });

  test("allows an active multi-page game section to stay collapsed until navigation changes", () => {
    render(
      <MemoryRouter initialEntries={["/valorant/stretch"]}>
        <SidebarNav />
      </MemoryRouter>,
    );
    const valorantGroup = screen.getByRole("button", { name: "nav.sectionValorant" });
    expect(valorantGroup.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(valorantGroup);
    expect(valorantGroup.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByRole("link", { name: "nav.valorantStretch" })).toBeNull();
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
