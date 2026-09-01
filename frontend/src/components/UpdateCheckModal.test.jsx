import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import UpdateCheckModal from "./UpdateCheckModal";

describe("UpdateCheckModal automatic install flow", () => {
  it("shows plain-language categories instead of technical release notes", () => {
    render(
      <UpdateCheckModal
        open
        info={{
          status: "available",
          latest_version: "3.0.1",
          auto_install: false,
          release_notes: "refactor(parser): replace eager payload allocation",
          user_release_notes: {
            fixed: ["修复大型 Demo 解析时容易卡顿的问题。"],
            added: ["新增 Demo 快速预览。"],
            optimized: ["降低大型 Demo 的内存占用。"],
          },
        }}
      />,
    );

    expect(screen.getByText(/修复问题|Bug fixes/)).toBeTruthy();
    expect(screen.getByText("修复大型 Demo 解析时容易卡顿的问题。")).toBeTruthy();
    expect(screen.getByText(/新增功能|New features/)).toBeTruthy();
    expect(screen.getByText(/性能与体验优化|Performance and experience/)).toBeTruthy();
    expect(screen.queryByText(/replace eager payload allocation/)).toBeNull();
  });

  it("falls back to legacy release notes when categorized notes are absent", () => {
    render(
      <UpdateCheckModal
        open
        info={{
          status: "available",
          latest_version: "3.0.0",
          auto_install: false,
          release_notes: "兼容旧版更新说明",
        }}
      />,
    );

    expect(screen.getByText("兼容旧版更新说明")).toBeTruthy();
  });

  it("always shows explicit update and skip actions for a normal update", () => {
    render(
      <UpdateCheckModal
        open
        info={{
          status: "available",
          current_version: "2.5.12",
          latest_version: "2.5.13",
          auto_install: true,
        }}
        onClose={vi.fn()}
        onConfirm={vi.fn()}
      />,
    );

    expect(screen.getByText(/发现新版本，是否立即更新|A new version is available. Update now\?/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /立即更新|Update now/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /暂不升级此版本|Skip this version/ })).toBeTruthy();
  });

  it("does not show a skip action for a force update, but still requires update confirmation", () => {
    const onClose = vi.fn();
    const onConfirm = vi.fn();
    render(
      <UpdateCheckModal
        open
        info={{
          status: "available",
          current_version: "2.5.12",
          latest_version: "2.5.13",
          update_mode: "force",
          auto_install: false,
        }}
        onClose={onClose}
        onConfirm={onConfirm}
      />,
    );

    expect(screen.getByText(/本次更新涉及重大内容|critical changes/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /立即更新|Update now/ })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /暂不升级此版本|Skip this version/ })).toBeNull();
  });

  it("makes the automatic installation handoff explicit", () => {
    render(
      <UpdateCheckModal
        open
        info={{
          status: "installing",
          current_version: "2.5.12",
          latest_version: "2.5.13",
          auto_install: true,
        }}
      />,
    );

    expect(screen.getByText(/自动覆盖安装|installing the update automatically/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /关闭|Close/ })).toBeNull();
  });
});
