import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { MemoryRouter } from "react-router-dom";
import HomePage from "./HomePage";

const { openExternal } = vi.hoisted(() => ({ openExternal: vi.fn() }));
vi.mock("../desktop/desktopBridge.js", () => ({ desktopBridge: { openExternal } }));
vi.mock("../i18n/useT.js", () => ({ useT: () => (key, params) => params ? `${key}:${params.version}` : key }));
vi.mock("../i18n/localeStore.js", () => ({ useLocaleStore: (selector) => selector({ effectiveLocale: "zh" }) }));

describe("HomePage", () => {
  beforeEach(() => openExternal.mockClear());

  test("presents cross-game release notes and feedback actions", () => {
    render(<MemoryRouter><HomePage /></MemoryRouter>);
    expect(screen.getByRole("heading", { name: "home.title" })).toBeTruthy();
    expect(screen.getByText("修复后台并行模式错误关闭 MaxGameStudio 主窗口的问题；现在只有节省内存模式会收起主程序。")).toBeTruthy();
    expect(screen.getByRole("note").textContent).toContain("home.stableReleaseNotice");
    expect(screen.getByRole("button", { name: /home.reportBug/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /home.requestFeature/ })).toBeTruthy();
  });

  test("opens localized issue forms and repository development links", () => {
    render(<MemoryRouter><HomePage /></MemoryRouter>);
    fireEvent.click(screen.getByRole("button", { name: /home.reportBug/ }));
    expect(openExternal).toHaveBeenCalledWith(expect.stringContaining("template=bug_report.yml"));
    fireEvent.click(screen.getByRole("button", { name: "home.viewPr" }));
    expect(openExternal).toHaveBeenCalledWith("https://github.com/INEEDBUG/MaxGameStudio/pulls");
  });
});
