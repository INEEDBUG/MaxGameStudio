import { render, waitFor, cleanup, fireEvent } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router-dom";
import { beforeEach, afterEach, it, expect, vi } from "vitest";
const mocks = vi.hoisted(() => ({ get: vi.fn(), ensure: vi.fn(), stream: vi.fn() }));
vi.mock("./api/api", () => ({ default: { get: mocks.get }, BACKEND_CONNECT_LABEL: "test", getDemosStreamUrl: () => "/stream" }));
vi.mock("./desktop/desktopBridge", () => ({ desktopBridge: { ensureBackend: mocks.ensure, onCloseChoiceRequested: async () => () => {} }, isDesktopApp: true }));
vi.mock("./components/LeagueRuntimeAutoManager", () => ({ default: () => null }));
vi.mock("./components/CustomTitleBar", () => ({ default: () => null }));
vi.mock("./pages/HomePage", () => ({ default: () => <div>HOME_READY</div> }));
vi.mock("./pages/LeagueRuntimePage", () => ({ default: () => <div>LEAGUE_READY</div> }));
vi.mock("./utils/shouldCheckAppUpdates", () => ({ AUTO_UPDATE_POLL_INTERVAL_MS: 900000, shouldCheckAppUpdates: async () => false }));
import App from "./App";
beforeEach(() => {
  vi.stubGlobal("__APP_VERSION__", "3.1.4-test");
  mocks.get.mockReset().mockImplementation(() => new Promise(() => {}));
  mocks.ensure.mockReset().mockImplementation(() => new Promise(() => {}));
  vi.stubGlobal("EventSource", mocks.stream);
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
it.each([["/", "HOME_READY"], ["/league", "LEAGUE_READY"]])("renders %s without Python or background HTTP", async (route, text) => {
  const view = render(<MemoryRouter initialEntries={[route]}><App /></MemoryRouter>);
  await waitFor(() => expect(view.getByText(text)).toBeTruthy());
  expect(mocks.ensure).not.toHaveBeenCalled();
  expect(mocks.get).not.toHaveBeenCalled();
  expect(mocks.stream).not.toHaveBeenCalled();
});
it("requests Python only on a service route, before rendering its page", async () => {
  render(<MemoryRouter initialEntries={["/valorant/stretch"]}><App /></MemoryRouter>);
  await waitFor(() => expect(mocks.ensure).toHaveBeenCalledTimes(1));
  expect(mocks.get).not.toHaveBeenCalled();
});
it("keeps the independent home available after Python startup fails", async () => {
  mocks.ensure.mockRejectedValue(new Error("TEST_BACKEND_UNAVAILABLE"));
  function NavigateHome() {
    const navigate = useNavigate();
    return <button onClick={() => navigate("/")}>TEST_GO_HOME</button>;
  }
  const view = render(<MemoryRouter initialEntries={["/valorant/stretch"]}><NavigateHome /><App /></MemoryRouter>);
  await waitFor(() => expect(view.getByText(/TEST_BACKEND_UNAVAILABLE/)).toBeTruthy());
  fireEvent.click(view.getByText("TEST_GO_HOME"));
  await waitFor(() => expect(view.getByText("HOME_READY")).toBeTruthy());
  expect(mocks.get).not.toHaveBeenCalled();
});
