import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import AboutPage from "./AboutPage";
import { HOME_RELEASE_NOTES, HOME_RELEASE_VERSION } from "../data/homeReleaseNotes.js";

const { openExternal } = vi.hoisted(() => ({ openExternal: vi.fn() }));
vi.mock("../desktop/desktopBridge.js", () => ({ desktopBridge: { openExternal } }));
vi.mock("../i18n/useT.js", () => ({ useT: () => (key, params) => params ? `${key}:${params.version}` : key }));
vi.mock("../i18n/localeStore.js", () => ({ useLocaleStore: (selector) => selector({ effectiveLocale: "zh" }) }));

describe("AboutPage", () => {
  beforeEach(() => openExternal.mockClear());

  test("presents cross-game release notes and feedback actions", () => {
    render(<MemoryRouter><AboutPage /></MemoryRouter>);
    expect(screen.getByRole("heading", { name: "nav.about" })).toBeTruthy();
    expect(screen.getByText(`v${HOME_RELEASE_VERSION}`)).toBeTruthy();
    expect(screen.getByText(HOME_RELEASE_NOTES.zh.fixed[0])).toBeTruthy();
    expect(screen.getByRole("note").textContent).toContain("home.betaReleaseNotice");
    expect(screen.getByRole("button", { name: /home.reportBug/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /home.requestFeature/ })).toBeTruthy();
  });

  test("opens localized issue forms and repository development links", () => {
    render(<MemoryRouter><AboutPage /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: /home.reportBug/ }));
    expect(openExternal).toHaveBeenCalledWith(expect.stringContaining("template=bug_report.yml"));
    fireEvent.click(screen.getByRole("button", { name: "home.viewPr" }));
    expect(openExternal).toHaveBeenCalledWith("https://github.com/INEEDBUG/MaxGameStudio/pulls");
  });
});
