import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import UpdateCheckModal from "./UpdateCheckModal";

describe("UpdateCheckModal automatic install flow", () => {
  it("shows automatic download without asking for a second confirmation", () => {
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

    expect(screen.getByText(/正在准备下载|Preparing download/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: /立即更新|Update now/ })).toBeNull();
    expect(screen.queryByRole("button", { name: /稍后再说|Later/ })).toBeNull();
  });

  it("offers to skip a normal automatic update during its grace window", () => {
    const onClose = vi.fn();
    render(
      <UpdateCheckModal
        open
        info={{
          status: "available",
          current_version: "2.5.12",
          latest_version: "2.5.13",
          auto_install: true,
          awaiting_choice: true,
        }}
        onClose={onClose}
      />,
    );

    expect(screen.getByText(/暂不升级此版本|Skip this version/)).toBeTruthy();
    expect(screen.getByText(/几秒后自动下载|download automatically in a few seconds/)).toBeTruthy();
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
