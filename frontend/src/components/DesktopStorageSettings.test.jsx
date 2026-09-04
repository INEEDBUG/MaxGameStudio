import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, test, vi } from "vitest";

const { bridge } = vi.hoisted(() => ({
  bridge: {
    getDesktopStorage: vi.fn(),
    chooseDesktopStorage: vi.fn(),
    cancelDesktopStorageChange: vi.fn(),
  },
}));

vi.mock("../desktop/desktopBridge.js", () => ({ desktopBridge: bridge }));

import DesktopStorageSettings from "./DesktopStorageSettings.jsx";

describe("DesktopStorageSettings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    bridge.getDesktopStorage.mockResolvedValue({ root: "D:\\MaxGameStudioData", system_drive: false });
  });

  test("loads current root and schedules a pending switch", async () => {
    bridge.chooseDesktopStorage.mockResolvedValue({
      root: "D:\\MaxGameStudioData",
      pending: "E:\\MaxGameStudioData",
      previous: null,
      system_drive: false,
      restart_required: true,
    });
    render(<DesktopStorageSettings />);

    expect(await screen.findByText(/D:\\MaxGameStudioData/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "更改存储位置" }));
    await waitFor(() => expect(bridge.chooseDesktopStorage).toHaveBeenCalledOnce());
    expect(await screen.findByText(/E:\\MaxGameStudioData/)).toBeTruthy();
    expect(screen.getByText(/下一次启动复制并校验/)).toBeTruthy();
  });

  test("does not invent a size when backend bytes are null", async () => {
    bridge.getDesktopStorage.mockResolvedValue({ root: "D:\\Data", bytes: null, system_drive: false });
    render(<DesktopStorageSettings />);
    expect(await screen.findByText(/D:\\Data/)).toBeTruthy();
    expect(screen.queryByText(/0 B/)).toBeNull();
  });

  test("cancels a pending switch successfully", async () => {
    bridge.getDesktopStorage.mockResolvedValue({ root: "D:\\Data", pending: "E:\\Data", restart_required: true });
    bridge.cancelDesktopStorageChange.mockResolvedValue({ root: "D:\\Data", pending: null, restart_required: false });
    render(<DesktopStorageSettings />);

    fireEvent.click(await screen.findByRole("button", { name: "取消待处理更改" }));
    await waitFor(() => expect(bridge.cancelDesktopStorageChange).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.queryByRole("button", { name: "取消待处理更改" })).toBeNull());
  });

  test("reports cancel errors without crashing", async () => {
    bridge.getDesktopStorage.mockResolvedValue({ root: "D:\\Data", pending: "E:\\Data", restart_required: true });
    bridge.cancelDesktopStorageChange.mockRejectedValue(new Error("storage busy"));
    render(<DesktopStorageSettings />);

    fireEvent.click(await screen.findByRole("button", { name: "取消待处理更改" }));
    expect((await screen.findByRole("alert")).textContent).toContain("storage busy");
  });

  test("reports refresh errors", async () => {
    bridge.getDesktopStorage
      .mockResolvedValueOnce({ root: "D:\\Data", system_drive: false })
      .mockRejectedValueOnce(new Error("refresh failed"));
    render(<DesktopStorageSettings />);
    await screen.findByText(/D:\\Data/);
    fireEvent.click(screen.getByRole("button", { name: "刷新存储状态" }));
    expect((await screen.findByRole("alert")).textContent).toContain("refresh failed");
  });

  test("ignores an in-flight response after unmount", async () => {
    let resolve;
    bridge.getDesktopStorage.mockReturnValue(new Promise((next) => { resolve = next; }));
    const { unmount } = render(<DesktopStorageSettings />);
    unmount();
    resolve({ root: "D:\\Data", bytes: null });
    await Promise.resolve();
    expect(screen.queryByText(/D:\\Data/)).toBeNull();
  });

  test("does not remain busy when StrictMode replays the mount effect", async () => {
    render(<StrictMode><DesktopStorageSettings /></StrictMode>);
    await screen.findByText(/D:\\MaxGameStudioData/);
    await waitFor(() => expect(screen.getByRole("button", { name: "更改存储位置" }).disabled).toBe(false));
  });
});
