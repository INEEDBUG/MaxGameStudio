import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useLocaleStore } from "../i18n/localeStore.js";
import {
  applyValorantStretch,
  confirmValorantStretch,
  decodeValorantCrosshair,
  encodeValorantCrosshair,
  fetchValorantCrosshair,
  fetchValorantDisplayStatus,
  prepareValorantStretch,
  restoreValorantStretch,
  saveValorantCrosshair,
  openValorantDeviceManager,
} from "../api/valorantLabApi.js";
import { DEFAULT_CROSSHAIR_PROFILES, serializeCrosshairCode } from "../utils/valorantLab.js";
import ValorantLabPage from "./ValorantLabPage.jsx";

vi.mock("../api/valorantLabApi.js", () => ({
  applyValorantStretch: vi.fn(),
  confirmValorantStretch: vi.fn(),
  decodeValorantCrosshair: vi.fn(),
  encodeValorantCrosshair: vi.fn(),
  fetchValorantCrosshair: vi.fn(),
  fetchValorantDisplayStatus: vi.fn(),
  isValorantLabApiUnavailable: vi.fn((error) => error?.code === "ERR_NETWORK" || [404, 503].includes(error?.response?.status)),
  openValorantDeviceManager: vi.fn(),
  prepareValorantStretch: vi.fn(),
  restoreValorantStretch: vi.fn(),
  saveValorantCrosshair: vi.fn(),
}));

const READY_STATUS = {
  overall: "ready",
  gpu: { status: "ready", name: "RTX test GPU" },
  monitor: { status: "ready", name: "Primary monitor" },
  refreshRate: { status: "ready", value: 240 },
};

describe("ValorantLabPage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useLocaleStore.getState().hydrate("zh");
    fetchValorantCrosshair.mockRejectedValue({ code: "ERR_NETWORK" });
    fetchValorantDisplayStatus.mockRejectedValue({ code: "ERR_NETWORK" });
    decodeValorantCrosshair.mockRejectedValue({ code: "ERR_NETWORK" });
    encodeValorantCrosshair.mockResolvedValue({ code: serializeCrosshairCode(DEFAULT_CROSSHAIR_PROFILES) });
    prepareValorantStretch.mockResolvedValue({});
    applyValorantStretch.mockResolvedValue({});
    confirmValorantStretch.mockResolvedValue({});
    restoreValorantStretch.mockResolvedValue({});
    openValorantDeviceManager.mockResolvedValue({ opened: true });
    saveValorantCrosshair.mockRejectedValue({ code: "ERR_NETWORK" });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockResolvedValue(undefined) },
    });
  });

  it("defaults to the community 1568×1080 preset and keeps real actions disabled when detection is unknown", async () => {
    render(<ValorantLabPage />);

    expect(screen.getByText("社区热门")).toBeTruthy();
    expect(screen.getAllByText("1568×1080").length).toBeGreaterThan(0);
    expect(screen.getByRole("button", { name: "应用真实拉伸" }).disabled).toBe(true);
    await waitFor(() => expect(screen.getAllByText("未知").length).toBeGreaterThan(0));
    expect(screen.getByText(/暂不可执行真实设置/)).toBeTruthy();
    expect(prepareValorantStretch).not.toHaveBeenCalled();
    expect(applyValorantStretch).not.toHaveBeenCalled();
  });

  it("requires a confirmed preview before the real stretch call", async () => {
    fetchValorantDisplayStatus.mockResolvedValue(READY_STATUS);
    render(<ValorantLabPage />);

    await waitFor(() => expect(screen.getByText("环境已确认，可以先预览设置。")).toBeTruthy());
    const previewButton = screen.getByRole("button", { name: "预览拉伸设置" });
    const applyButton = screen.getByRole("button", { name: "应用真实拉伸" });
    expect(previewButton.disabled).toBe(false);
    expect(applyButton.disabled).toBe(true);

    fireEvent.click(previewButton);
    await waitFor(() => expect(prepareValorantStretch).toHaveBeenCalledWith(expect.objectContaining({ width: 1568, height: 1080, preset: "1568x1080" })));
    expect(screen.getByText("预览已生成；确认下方风险说明后才可应用。")).toBeTruthy();
    expect(applyButton.disabled).toBe(true);

    fireEvent.click(screen.getByRole("checkbox", { name: /我已确认游戏会短暂切换/ }));
    expect(applyButton.disabled).toBe(false);
    fireEvent.click(applyButton);
    await waitFor(() => expect(applyValorantStretch).toHaveBeenCalledWith(expect.objectContaining({ width: 1568, height: 1080, confirmed: true })));
  });

  it("keeps P/A/S profiles separate and supports copying the generated code", async () => {
    render(<ValorantLabPage />);

    expect(screen.getByRole("tab", { name: "P" }).getAttribute("aria-selected")).toBe("true");
    fireEvent.click(screen.getByRole("tab", { name: "A" }));
    expect(screen.getByRole("tab", { name: "A" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.getByText("瞄准")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "红色" }));
    fireEvent.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringMatching(/^0;s;1;P;/)));
    expect(screen.getByText("已复制配置代码")).toBeTruthy();
  });

  it("skips the monitor-disable step when detection proves all physical monitors are already disabled", async () => {
    fetchValorantDisplayStatus.mockResolvedValue({
      overall: "ready",
      gpu: { status: "ready", name: "RTX test GPU" },
      monitor: { status: "ready", name: "All physical monitors disabled" },
      refreshRate: { status: "ready", value: 240 },
      raw_monitor_status: "all_present_physical_monitors_disabled",
      monitor_disable_status: "all_present_physical_monitors_disabled",
      safe_to_skip_disable: true,
    });
    render(<ValorantLabPage />);

    await waitFor(() => expect(screen.getByText("环境已确认，可以先预览设置。")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "预览拉伸设置" }));
    await waitFor(() => expect(prepareValorantStretch).toHaveBeenCalledWith(expect.objectContaining({ width: 1568, height: 1080 })));
    expect(screen.getByText("物理监视器已禁用")).toBeTruthy();
    expect(screen.getByText("已禁用，已跳过")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "打开设备管理器" })).toBeNull();
  });

  it("offers Device Manager as a manual helper when the monitor prerequisite is not met", async () => {
    fetchValorantDisplayStatus.mockResolvedValue({
      overall: "warning",
      gpu: { status: "ready", name: "RTX test GPU" },
      monitor: { status: "warning", name: "Primary monitor" },
      refreshRate: { status: "ready", value: 240 },
      raw_monitor_status: "some_present_physical_monitors_enabled",
      safe_to_skip_disable: false,
    });
    render(<ValorantLabPage />);

    await waitFor(() => expect(screen.getByRole("button", { name: "打开设备管理器" })).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "打开设备管理器" }));
    await waitFor(() => expect(openValorantDeviceManager).toHaveBeenCalledTimes(1));
    expect(screen.getByText(/设备管理器已打开/)).toBeTruthy();
  });

  it("imports the exported JSON profile as well as the native share code", async () => {
    const { container } = render(<ValorantLabPage />);
    const input = container.querySelector('input[type="file"]');
    const json = JSON.stringify({ version: 1, profiles: { P: { color: "red", innerLines: false } } });
    const file = new File([json], "profile.json", { type: "application/json" });
    Object.defineProperty(file, "text", { configurable: true, value: async () => json });
    fireEvent.change(input, { target: { files: [file], value: "" } });
    await waitFor(() => expect(screen.getByText("配置已导入")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith(expect.stringMatching(/^0;s;1;P;/)));

    const native = serializeCrosshairCode(DEFAULT_CROSSHAIR_PROFILES);
    const nativeFile = new File([native], "profile.txt", { type: "text/plain" });
    Object.defineProperty(nativeFile, "text", { configurable: true, value: async () => native });
    fireEvent.change(input, { target: { files: [nativeFile], value: "" } });
    await waitFor(() => expect(screen.getByText("配置已导入")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith(native));
  });

  it("uses backend decode and response codes in online mode", async () => {
    const initialCode = serializeCrosshairCode(DEFAULT_CROSSHAIR_PROFILES);
    const importedCode = `${initialCode};future_field;keep-me`;
    const encodedCode = `${initialCode};P;future_field;encoded-away`;
    const savedCode = `${initialCode};future_field;saved-by-backend`;
    fetchValorantCrosshair.mockResolvedValue({ profiles: DEFAULT_CROSSHAIR_PROFILES, code: initialCode });
    decodeValorantCrosshair.mockResolvedValue({ profiles: DEFAULT_CROSSHAIR_PROFILES, code: importedCode });
    encodeValorantCrosshair.mockResolvedValue({ code: encodedCode });
    saveValorantCrosshair.mockResolvedValue({ profiles: DEFAULT_CROSSHAIR_PROFILES, code: savedCode, saved: true });

    const { container } = render(<ValorantLabPage />);
    await waitFor(() => expect(screen.getByText("后端严格模式")).toBeTruthy());

    const input = container.querySelector('input[type="file"]');
    const file = new File([importedCode], "profile.txt", { type: "text/plain" });
    Object.defineProperty(file, "text", { configurable: true, value: async () => importedCode });
    fireEvent.change(input, { target: { files: [file], value: "" } });

    await waitFor(() => expect(decodeValorantCrosshair).toHaveBeenCalledWith(importedCode));
    fireEvent.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith(importedCode));

    fireEvent.click(screen.getByRole("button", { name: "红色" }));
    await waitFor(() => expect(encodeValorantCrosshair).toHaveBeenCalledWith(expect.objectContaining({ P: expect.objectContaining({ color: "red" }) })));
    await waitFor(() => expect(screen.getByText(/当前代码可能包含编辑器未暴露/)).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith(encodedCode));

    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    await waitFor(() => expect(saveValorantCrosshair).toHaveBeenCalledWith(expect.objectContaining({ P: expect.objectContaining({ color: "red" }) })));
    fireEvent.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith(savedCode));
  });

  it("labels local fallback and keeps an imported native code intact until editing", async () => {
    const native = `${serializeCrosshairCode(DEFAULT_CROSSHAIR_PROFILES)};future_field;keep-me`;
    const { container } = render(<ValorantLabPage />);
    await waitFor(() => expect(screen.getByText("本机回退模式")).toBeTruthy());

    const input = container.querySelector('input[type="file"]');
    const file = new File([native], "profile.txt", { type: "text/plain" });
    Object.defineProperty(file, "text", { configurable: true, value: async () => native });
    fireEvent.change(input, { target: { files: [file], value: "" } });
    await waitFor(() => expect(screen.getByText("配置已导入")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "复制" }));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith(native));

    fireEvent.click(screen.getByRole("button", { name: "红色" }));
    await waitFor(() => expect(screen.getByText(/当前代码可能包含编辑器未暴露/)).toBeTruthy());
  });

  it("shows the safety countdown and supports restore or confirm after applying", async () => {
    fetchValorantDisplayStatus.mockResolvedValue(READY_STATUS);
    applyValorantStretch.mockResolvedValue({ rollback_deadline: Date.now() / 1000 + 20 });
    render(<ValorantLabPage />);

    await waitFor(() => expect(screen.getByText("环境已确认，可以先预览设置。")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "预览拉伸设置" }));
    await waitFor(() => expect(screen.getByText("预览已生成；确认下方风险说明后才可应用。")).toBeTruthy());
    fireEvent.click(screen.getByRole("checkbox", { name: /我已确认游戏会短暂切换/ }));
    fireEvent.click(screen.getByRole("button", { name: "应用真实拉伸" }));

    await waitFor(() => expect(screen.getByText("显示模式安全确认")).toBeTruthy());
    expect(screen.getByText(/若画面正常请选择保留/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "立即恢复" }));
    await waitFor(() => expect(restoreValorantStretch).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByText("已恢复切换前的显示模式。")).toBeTruthy());

    applyValorantStretch.mockResolvedValue({ rollback_deadline: Date.now() / 1000 + 20 });
    fireEvent.click(screen.getByRole("button", { name: "预览拉伸设置" }));
    await waitFor(() => expect(screen.getByText("预览已生成；确认下方风险说明后才可应用。")).toBeTruthy());
    fireEvent.click(screen.getByRole("checkbox", { name: /我已确认游戏会短暂切换/ }));
    fireEvent.click(screen.getByRole("button", { name: "应用真实拉伸" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "画面正常，保留至本次会话" })).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "画面正常，保留至本次会话" }));
    await waitFor(() => expect(confirmValorantStretch).toHaveBeenCalledTimes(1));
  });
});
