# Cosmetic workspace scroll regression

This real-browser test renders the production workspace/loadout/styles in the
fixed-height, overflow-hidden app shell layout. Only the API boundary is replaced
with deterministic inventory/sky fixtures. No backend, save, game or recording
action is allowed. It catches wheel/keyboard/bottom-access and picker-scroll
regressions which jsdom does not model.

Use existing Playwright and Chrome installations (no product dependency added):

1. Start Vite on localhost: `node node_modules/vite/bin/vite.js --host 127.0.0.1 --port 18746 --strictPort`.
2. Set `PLAYWRIGHT_MODULE` to an installed `playwright/index.mjs` if Playwright is
   not resolvable locally; set `SCROLL_TEST_CHROME` to an installed Chrome binary.
   Keep `TEMP`/`TMP` on the approved test disk for the isolated browser profile.
3. Run `node tests/browser/cosmetic-scroll.browser.mjs`.

Optional `SCROLL_TEST_URL` selects the local Vite server. Tests cover 1440x900 and
1100x700 in dark/light modes. Browser contexts are isolated and closed in the
test; no daily user profile or login state is imported. Close the owned Vite
server after testing. The fixture is not a production entry point.

## Recording queue cosmetics

Run `queue-cosmetic.browser.mjs` against Vite port 18747 (`QUEUE_TEST_URL`
overrides it), using the same installed Playwright/Chrome environment variables.
The fixture uses the real inspector, store and lazy loadout dialog. API-only
fixtures check categories, CT/T, select/equip/save/reopen/cancel/reset/share,
missing-player rejection, focus restoration, independent scrolling and visible
buttons in dark/light 1440x900 and 1100x700 layouts. No native actions permitted.
