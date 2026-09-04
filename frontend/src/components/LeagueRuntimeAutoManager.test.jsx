import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router-dom";
import { beforeEach, describe, expect, test, vi } from "vitest";
import LeagueRuntimeAutoManager from "./LeagueRuntimeAutoManager.jsx";

const mocks = vi.hoisted(() => ({ launch: vi.fn(), subscribe: vi.fn(), readPreference: vi.fn() }));
vi.mock("../desktop/desktopBridge.js", () => ({ isDesktopApp: true }));
vi.mock("../utils/leagueStartupPreference.js", () => ({ readLeagueStartupPreference: mocks.readPreference }));
vi.mock("../utils/leagueRuntimeLaunchCoordinator.js", () => ({
  clearHandledLeagueSession: vi.fn(),
  leagueClientSessionId: (status) => status?.connected && status?.client_pid ? `pid:${status.client_pid}` : "",
  launchLeagueRuntimeCoordinated: mocks.launch,
}));
vi.mock("../utils/leagueLabStatusSubscription.js", () => ({ subscribeLeagueLabStatus: mocks.subscribe }));

describe("LeagueRuntimeAutoManager", () => {
  beforeEach(() => {
    mocks.launch.mockReset().mockResolvedValue({ launched: true });
    mocks.readPreference.mockReset().mockReturnValue({ mode: "memory", remembered: true });
    mocks.subscribe.mockReset();
  });

  test("launches once when a remembered mode sees a League client", async () => {
    let listener;
    const unsubscribe = vi.fn();
    mocks.subscribe.mockImplementation((next) => { listener = next; return unsubscribe; });
    const view = render(<MemoryRouter initialEntries={["/settings"]}><LeagueRuntimeAutoManager /></MemoryRouter>);
    act(() => listener({ connected: true, client_pid: 41 }));
    await waitFor(() => expect(mocks.launch).toHaveBeenCalledWith("memory", { sessionId: "pid:41" }));
    view.unmount();
    expect(unsubscribe).toHaveBeenCalledTimes(1);
  });

  test("does not auto-launch while the preference is ask", async () => {
    let listener;
    mocks.readPreference.mockReturnValue({ mode: "ask", remembered: true });
    mocks.subscribe.mockImplementation((next) => { listener = next; return vi.fn(); });
    render(<MemoryRouter initialEntries={["/settings"]}><LeagueRuntimeAutoManager /></MemoryRouter>);
    act(() => listener({ connected: true, client_pid: 42 }));
    await Promise.resolve();
    expect(mocks.launch).not.toHaveBeenCalled();
  });

  test("forwards a remembered administrator hint during automatic entry", async () => {
    let listener;
    mocks.readPreference.mockReturnValue({ mode: "parallel", remembered: true, administrator: true });
    mocks.subscribe.mockImplementation((next) => { listener = next; return vi.fn(); });
    render(<MemoryRouter initialEntries={["/settings"]}><LeagueRuntimeAutoManager /></MemoryRouter>);
    act(() => listener({ connected: true, client_pid: 45 }));
    await waitFor(() => expect(mocks.launch).toHaveBeenCalledWith("parallel", { sessionId: "pid:45", administrator: true }));
  });

  test("does not auto-launch while the League route is open", async () => {
    let listener;
    mocks.subscribe.mockImplementation((next) => { listener = next; return vi.fn(); });
    render(<MemoryRouter initialEntries={["/league"]}><LeagueRuntimeAutoManager /></MemoryRouter>);
    act(() => listener({ connected: true, client_pid: 43 }));
    await Promise.resolve();
    expect(mocks.launch).not.toHaveBeenCalled();
  });

  test("resumes automatic entry after leaving the League route", async () => {
    let listener;
    mocks.subscribe.mockImplementation((next) => { listener = next; return vi.fn(); });
    function LeaveRouteButton() {
      const navigate = useNavigate();
      return <button type="button" onClick={() => navigate("/settings")}>Leave League</button>;
    }
    render(<MemoryRouter initialEntries={["/league"]}><LeaveRouteButton /><LeagueRuntimeAutoManager /></MemoryRouter>);
    act(() => listener({ connected: true, client_pid: 44 }));
    expect(mocks.launch).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Leave League" }));
    act(() => listener({ connected: true, client_pid: 44 }));
    await waitFor(() => expect(mocks.launch).toHaveBeenCalledWith("memory", { sessionId: "pid:44" }));
  });
});
