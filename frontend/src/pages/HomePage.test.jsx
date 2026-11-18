import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, test, vi } from "vitest";
import HomePage from "./HomePage.jsx";
vi.mock("../i18n/useT.js", () => ({ useT: () => (key) => key }));
vi.mock("../i18n/localeStore.js", () => ({ useLocaleStore: (selector) => selector({ effectiveLocale: "zh" }) }));
vi.mock("../components/SteamAccountsPanel.jsx", () => ({ default: () => <section aria-label="Steam 账户" /> }));
describe("HomePage", () => {
  test("puts account switching and existing tools within one click", () => {
    render(<MemoryRouter><HomePage /></MemoryRouter>);
    expect(screen.getByRole("region", { name: "Steam 账户" })).toBeTruthy();
    for (const path of ["/cs2/library", "/cs2/match-history", "/cs2/lite-cut", "/valorant/stretch", "/peripherals/sensitivity"]) {
      expect(document.querySelector(`a[href="${path}"]`)).toBeTruthy();
    }
    expect(document.querySelector('a[href="/league"]')).toBeNull();
  });
});
