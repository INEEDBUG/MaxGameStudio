import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, test, vi } from "vitest";
import LeagueRuntimePage from "./LeagueRuntimePage";
import { clearLeagueStartupPreference, readLeagueStartupPreference, writeLeagueStartupPreference } from "../utils/leagueStartupPreference.js";

const { getStatus, launch, prepare, cancelPrewarm } = vi.hoisted(() => ({ getStatus: vi.fn(), launch: vi.fn(), prepare: vi.fn(), cancelPrewarm: vi.fn() }));
vi.mock("../desktop/desktopBridge.js", () => ({
  isDesktopApp: true,
  desktopBridge: { getLeagueRuntimeStatus: getStatus, launchLeagueRuntime: launch, prepareLeagueRuntime: prepare, cancelLeaguePrewarm: cancelPrewarm },
}));
vi.mock("../i18n/useT.js", () => ({ useT: () => (key) => ({
  "leagueRuntime.title": "英雄联盟工作台", "leagueRuntime.subtitle": "选择启动方式", "leagueRuntime.chooseMode": "选择启动方式", "leagueRuntime.explicitLaunch": "需要点击", "leagueRuntime.modeAsk": "每次询问", "leagueRuntime.modeAskDesc": "每次询问", "leagueRuntime.modeMemory": "节省内存", "leagueRuntime.modeMemoryDesc": "节省内存", "leagueRuntime.modeParallel": "后台并行", "leagueRuntime.modeParallelDesc": "后台并行", "leagueRuntime.remember": "记住这些选择", "leagueRuntime.administrator": "以管理员权限启动", "leagueRuntime.administratorHint": "每次仍显示 UAC", "leagueRuntime.administratorUnavailable": "需要重新安装完整安装包", "leagueRuntime.adminCancelled": "已取消管理员启动", "leagueRuntime.launchInFlight": "另一项启动选择正在处理中，请等待它完成后再试。", "leagueRuntime.clear": "清除选择", "leagueRuntime.launch": "启动工作台", "leagueRuntime.launching": "正在启动", "leagueRuntime.refresh": "刷新状态", "leagueRuntime.loading": "加载中", "leagueRuntime.error": "失败", "leagueRuntime.unavailable": "不可用", "leagueRuntime.desktopOnly": "仅桌面", "leagueRuntime.statusTitle": "运行状态", "leagueRuntime.runtimeStatus": "工作台", "leagueRuntime.activePrivilege": "当前权限", "leagueRuntime.privilegeAdministrator": "管理员", "leagueRuntime.privilegeStandard": "普通用户", "leagueRuntime.active": "运行中", "leagueRuntime.stopped": "未启动", "leagueRuntime.hostMemory": "宿主工作集", "leagueRuntime.expectedMemory": "预计内存", "leagueRuntime.safety": "安全说明", "leagueRuntime.prewarmHint": "预热不连接客户端，也不运行自动化；会临时占用内存，准备就绪后可更快打开。", "leagueRuntime.prewarmReady": "已准备就绪", "leagueRuntime.prewarmPreparing": "正在准备", "leagueRuntime.prewarmAdministratorHint": "管理员模式需要点击 UAC，不进行普通模式预热。",
}[key] || key) }));

describe("LeagueRuntimePage", () => {
  test("retries a dead prewarm once and reports asynchronous launch errors", async () => {
    let poll;
    const originalInterval = window.setInterval;
    const interval = vi.spyOn(window, "setInterval").mockImplementation((callback, delay, ...args) => {
      if (delay === 1000) { poll = callback; return 778; }
      return originalInterval(callback, delay, ...args);
    });
    let page;
    try {
      page = render(<LeagueRuntimePage />);
      await waitFor(() => expect(prepare).toHaveBeenCalledTimes(1));
      getStatus.mockResolvedValue({ available: true, active: false, prewarm_exited: true, prewarm_ready: false });
      await act(async () => { poll(); });
      await waitFor(() => expect(prepare).toHaveBeenCalledTimes(2));
      await act(async () => { poll(); });
      expect(prepare).toHaveBeenCalledTimes(2);
      getStatus.mockResolvedValue({ available: true, active: false, last_error: "工作台未能显示，主窗口保持打开，请重试" });
      await act(async () => { poll(); });
      expect(screen.getByRole("alert").textContent).toContain("工作台未能显示");
    } finally { page?.unmount(); interval.mockRestore(); }
  });

  test("resumes hidden prewarm after a parallel workbench exits", async () => {
    let poll;
    const originalInterval = window.setInterval;
    const interval = vi.spyOn(window, "setInterval").mockImplementation((callback, delay, ...args) => {
      if (delay === 1000) { poll = callback; return 777; }
      return originalInterval(callback, delay, ...args);
    });
    let page;
    try {
      getStatus.mockResolvedValue({ available: true, active: true });
      page = render(<LeagueRuntimePage />);
      await waitFor(() => expect(screen.getByText("运行中")).toBeTruthy());
      expect(prepare).not.toHaveBeenCalled();
      expect(poll).toBeTypeOf("function");
      getStatus.mockResolvedValue({ available: true, active: false, prewarm_ready: false });
      await act(async () => { poll(); });
      await waitFor(() => expect(prepare).toHaveBeenCalled());
      expect(screen.getByText("未启动")).toBeTruthy();
    } finally { page?.unmount(); interval.mockRestore(); }
  });

  beforeEach(() => { window.localStorage.clear(); getStatus.mockReset(); launch.mockReset(); prepare.mockReset(); cancelPrewarm.mockReset(); getStatus.mockResolvedValue({ available: true, administrator_available: true, active: false, prewarm_ready: false, host_working_set_bytes: 1048576 }); prepare.mockResolvedValue({ prewarm_ready: true }); cancelPrewarm.mockResolvedValue({ prewarm_ready: false }); });

  test("prewarms ordinary mode and cancels it when leaving for administrator mode", async () => {
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(prepare).toHaveBeenCalled());
    fireEvent.click(screen.getAllByRole("radio")[2]);
    fireEvent.click(screen.getByRole("checkbox", { name: "以管理员权限启动" }));
    await waitFor(() => expect(cancelPrewarm).toHaveBeenCalled());
    expect(launch).not.toHaveBeenCalled();
  });

  test("loads status without automatically launching and requires a mode click", async () => {
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(screen.getByText("运行状态")).toBeTruthy());
    expect(getStatus).toHaveBeenCalledTimes(1);
    expect(launch).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "启动工作台" }).disabled).toBe(true);
    fireEvent.click(screen.getAllByRole("radio")[1]);
    expect(screen.getByRole("button", { name: "启动工作台" }).disabled).toBe(false);
  });

  test("shares the settings preference and clears it through the shared utility", async () => {
    writeLeagueStartupPreference("ask");
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(screen.getByText("运行状态")).toBeTruthy());
    expect(screen.getAllByRole("radio")[0].getAttribute("aria-checked")).toBe("true");
    fireEvent.click(screen.getByRole("button", { name: "清除选择" }));
    expect(readLeagueStartupPreference()).toBeNull();
    expect(screen.getAllByRole("radio")[0].getAttribute("aria-checked")).toBe("true");
  });

  test("uses a remembered mode as the default without launching on route entry", async () => {
    writeLeagueStartupPreference("parallel");
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(screen.getByText("运行状态")).toBeTruthy());
    expect(screen.getAllByRole("radio")[2].getAttribute("aria-checked")).toBe("true");
    expect(launch).not.toHaveBeenCalled();
  });

  test("restores the remembered administrator hint and forwards it on explicit launch", async () => {
    writeLeagueStartupPreference("parallel", true, localStorage, { administrator: true });
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(screen.getByText("运行状态")).toBeTruthy());
    expect(screen.getByRole("checkbox", { name: "以管理员权限启动" }).checked).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "启动工作台" }));
    await waitFor(() => expect(launch).toHaveBeenCalledWith("parallel", { administrator: true }));
  });

  test("does not remember an administrator launch when UAC is cancelled", async () => {
    launch.mockRejectedValueOnce(new Error("UAC_CANCELLED: cancelled"));
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(screen.getByText("运行状态")).toBeTruthy());
    fireEvent.click(screen.getAllByRole("radio")[2]);
    fireEvent.click(screen.getByRole("checkbox", { name: "以管理员权限启动" }));
    fireEvent.click(screen.getByRole("button", { name: "启动工作台" }));
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("已取消管理员启动"));
    expect(readLeagueStartupPreference()).toBeNull();
  });

  test("surfaces a protected administrator preparation failure after the host is restored", async () => {
    getStatus.mockResolvedValueOnce({
      available: true,
      administrator_available: true,
      active: false,
      last_error: "管理员工作台的受保护运行时准备失败",
    });
    render(<LeagueRuntimePage />);
    await waitFor(() => {
      expect(screen.getByRole("alert").textContent).toContain("受保护运行时准备失败");
    });
  });

  test("blocks administrator launch without the protected chain but lets a remembered choice be cleared", async () => {
    writeLeagueStartupPreference("parallel", true, localStorage, { administrator: true });
    getStatus.mockResolvedValueOnce({ available: true, administrator_available: false, active: false });
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(screen.getByText("运行状态")).toBeTruthy());
    const checkbox = screen.getByRole("checkbox", { name: "以管理员权限启动" });
    expect(checkbox.checked).toBe(true);
    expect(checkbox.disabled).toBe(false);
    expect(screen.getByRole("button", { name: "启动工作台" }).disabled).toBe(true);
    fireEvent.click(checkbox);
    expect(checkbox.checked).toBe(false);
    expect(screen.getByRole("button", { name: "启动工作台" }).disabled).toBe(false);
    expect(screen.getByText("需要重新安装完整安装包")).toBeTruthy();
  });

  test.each(["memory", "parallel"])("passes %s to the desktop launch command", async (selectedMode) => {
    render(<LeagueRuntimePage />);
    await waitFor(() => expect(screen.getByText("运行状态")).toBeTruthy());
    fireEvent.click(screen.getAllByRole("radio")[selectedMode === "memory" ? 1 : 2]);
    fireEvent.click(screen.getByRole("button", { name: "启动工作台" }));
    await waitFor(() => expect(launch).toHaveBeenCalledWith(selectedMode));
    clearLeagueStartupPreference();
  });
});
