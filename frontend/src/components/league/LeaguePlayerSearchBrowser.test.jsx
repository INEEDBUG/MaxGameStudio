import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import LeaguePlayerSearchBrowser from "./LeaguePlayerSearchBrowser";

describe("LeaguePlayerSearchBrowser", () => {
  it("opens, pins and deletes history and explicitly launches a spectatable friend", () => {
    const onOpen = vi.fn();
    const onPin = vi.fn();
    const onDelete = vi.fn();
    const onSpectate = vi.fn();
    render(<LeaguePlayerSearchBrowser
      history={[{ puuid: "recent", game_name: "Recent", tag_line: "CN1", server_id: "hn1", pinned: false, profile_icon_id: 1 }]}
      friends={[{ puuid: "friend", game_name: "Friend", tag_line: "CN2", availability: "dnd", game_status: "inGame", spectatable: true, profile_icon_id: 2 }]}
      onOpen={onOpen}
      onPin={onPin}
      onDelete={onDelete}
      onSpectate={onSpectate}
    />);

    fireEvent.click(screen.getByRole("button", { name: /Recent/ }));
    fireEvent.click(screen.getByRole("button", { name: "置顶最近访问" }));
    fireEvent.click(screen.getByRole("button", { name: "删除最近访问" }));
    fireEvent.click(screen.getByRole("button", { name: "观战 Friend" }));

    expect(onOpen).toHaveBeenCalledWith("recent", "hn1");
    expect(onPin).toHaveBeenCalledWith("recent", true, "hn1");
    expect(onDelete).toHaveBeenCalledWith("recent", "hn1");
    expect(onSpectate).toHaveBeenCalledWith("friend");
  });

  it("masks Riot IDs in streamer mode", () => {
    render(<LeaguePlayerSearchBrowser
      history={[{ puuid: "recent", game_name: "SecretName", tag_line: "SECRET" }]}
      friends={[]}
      streamerMode
      useAliases
      onOpen={vi.fn()}
      onPin={vi.fn()}
      onDelete={vi.fn()}
      onSpectate={vi.fn()}
    />);

    expect(screen.queryByText("SecretName")).toBeNull();
    expect(screen.queryByText(/SECRET/)).toBeNull();
    expect(screen.getByLabelText("筛选最近访问").disabled).toBe(true);
  });
});
