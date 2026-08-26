import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import LeagueMatchFilterPresets from "./LeagueMatchFilterPresets";

const FILTER = { result: "win", mode: "CLASSIC", position: "jungle", text: "", minKills: "8", maxDeaths: "", minKda: "3" };

describe("LeagueMatchFilterPresets", () => {
  beforeEach(() => localStorage.clear());

  it("saves, restores and deletes a named local filter", () => {
    const onApply = vi.fn();
    render(<LeagueMatchFilterPresets filter={FILTER} onApply={onApply} />);
    fireEvent.change(screen.getByPlaceholderText(/为当前筛选命名/), { target: { value: "打野高光" } });
    fireEvent.click(screen.getByRole("button", { name: "保存筛选" }));

    fireEvent.click(screen.getByRole("button", { name: "打野高光" }));
    expect(onApply).toHaveBeenCalledWith(FILTER);
    expect(JSON.parse(localStorage.getItem("league-player-filter-presets"))).toHaveLength(1);

    fireEvent.click(screen.getByRole("button", { name: "删除筛选 打野高光" }));
    expect(screen.queryByRole("button", { name: "打野高光" })).toBeNull();
    expect(JSON.parse(localStorage.getItem("league-player-filter-presets"))).toEqual([]);
  });
});
