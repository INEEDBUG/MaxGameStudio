import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const stagingScriptPath = path.resolve(
  __dirname,
  "../../../packaging/windows/stage-league-runtime.ps1",
);
const stagingScript = fs.readFileSync(stagingScriptPath, "utf8");
const pinnedSourceRoot =
  process.env.MAXGAMESTUDIO_LEAGUE_PINNED_SOURCE ||
  process.env.MAXGAMESTUDIO_LEAGUE_SOURCE_DIR ||
  "";
const pinnedIntegrationTest = pinnedSourceRoot ? it : it.skip;

function readPinned(relativePath) {
  return fs.readFileSync(path.join(pinnedSourceRoot, relativePath), "utf8");
}

function extractMethod(source, signature) {
  const signatureIndex = source.indexOf(signature);
  if (signatureIndex < 0) throw new Error(`Missing pinned method ${signature}`);
  const bodyStart = source.indexOf("{", signatureIndex);
  let depth = 0;
  for (let index = bodyStart; index < source.length; index += 1) {
    if (source[index] === "{") depth += 1;
    if (source[index] === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(bodyStart + 1, index);
    }
  }
  throw new Error(`Unclosed pinned method ${signature}`);
}

function compileMethod(source, signature, parameters = "") {
  const body = extractMethod(source, signature).replace(/!([.;),])/g, "$1");
  return vm.runInNewContext(`(function (${parameters}) {${body}\n})`, {
    process: { platform: "win32" },
  });
}

function createCloseMock(closeAction) {
  const calls = [];
  return {
    calls,
    value: {
      _trueClose: false,
      _nextCloseAction: null,
      _context: {
        shared: { global: { isReadyToQuit: false } },
        ipc: { sendEvent: (...args) => calls.push(["sendEvent", ...args]) },
      },
      _namespace: "main-window",
      settings: { closeAction },
      state: { show: false },
      _window: {
        hide: () => calls.push(["hide"]),
        show: () => calls.push(["show"]),
      },
      showOrRestore: () => calls.push(["showOrRestore"]),
      close: (...args) => calls.push(["close", ...args]),
      emit: (...args) => calls.push(["emit", ...args]),
    },
  };
}

function createCloseEvent() {
  const calls = [];
  return {
    calls,
    event: { preventDefault: () => calls.push("preventDefault") },
  };
}

describe("embedded League runtime close staging contract", () => {
  it("uses the system tray while preserving the upstream close confirmation", () => {
    expect(stagingScript).not.toContain("$embeddedCloseGuard");
    expect(stagingScript).not.toContain("$embeddedCloseMinimize");
    expect(stagingScript).toContain("this._window?.hide()");
    expect(stagingScript).toContain("$closeStrategyAnchor");
    expect(stagingScript).toContain("Hide to system tray");
    expect(stagingScript).toContain("manager.use(TrayMain)");
    expect(stagingScript).toContain("onActivated(() => {");
    expect(stagingScript).toContain("if (this._disposed) return");
    expect(stagingScript).toContain("this._disposed = true");
  });

  it("keeps the two close choices branded for both supported locales", () => {
    expect(stagingScript).toContain("Close League Workspace");
    expect(stagingScript).toContain("Hide to system tray");
    expect(stagingScript).toContain("Return to MaxGameStudio");
    expect(stagingScript).toContain("$zhCloseLeagueWorkspace");
    expect(stagingScript).toContain("$zhHideToSystemTray");
    expect(stagingScript).toContain("$zhReturnToMaxGameStudio");
    expect(stagingScript).toContain(
      "$zhQuitCommonAppName = (& $makeUnicodeText @(0x9000, 0x51FA)) + ' $t(common:appName)'",
    );
  });

  it("keeps the staging script ASCII-only and constructs Chinese labels at build time", () => {
    expect(
      [...stagingScript].filter((character) => character.charCodeAt(0) > 0x7f),
    ).toEqual([]);
    expect(stagingScript).toContain("$makeUnicodeText");
  });

  pinnedIntegrationTest(
    "executes the pinned main-window close branches with hide, ask, and quit behavior",
    () => {
      const source = readPinned(
        "src/main/shards/window-manager/main-window/window.ts",
      );
      const handleClose = compileMethod(
        source,
        "protected override handleClose(event: Event)",
        "event",
      );

      const tray = createCloseMock("minimize-to-tray");
      const trayEvent = createCloseEvent();
      handleClose.call(tray.value, trayEvent.event);
      expect(tray.calls).toEqual([["hide"]]);
      expect(trayEvent.calls).toEqual(["preventDefault"]);

      const ask = createCloseMock("ask");
      const askEvent = createCloseEvent();
      handleClose.call(ask.value, askEvent.event);
      expect(ask.calls).toEqual([
        ["show"],
        ["sendEvent", "main-window", "close-asking"],
        ["showOrRestore"],
      ]);
      expect(askEvent.calls).toEqual(["preventDefault"]);

      const quit = createCloseMock("quit");
      const quitEvent = createCloseEvent();
      handleClose.call(quit.value, quitEvent.event);
      expect(quit.calls).toEqual([["close", true]]);
      expect(quitEvent.calls).toEqual([]);
    },
  );

  pinnedIntegrationTest(
    "executes the pinned tray restore, quit, and disposal callbacks",
    () => {
      const source = readPinned("src/main/shards/tray/tray-menu-controller.ts");
      const calls = [];
      const trayInstances = [];
      const menus = [];
      const Menu = {
        buildFromTemplate: (template) => {
          menus.push(template);
          return template;
        },
        setApplicationMenu: (menu) => calls.push(["setApplicationMenu", menu]),
      };
      class MenuItem {
        constructor(options) {
          Object.assign(this, options);
        }
      }
      class Tray {
        constructor() {
          this.listeners = new Map();
          trayInstances.push(this);
        }
        setToolTip(value) {
          calls.push(["setToolTip", value]);
        }
        addListener(name, callback) {
          this.listeners.set(name, callback);
        }
        popUpContextMenu(menu) {
          calls.push(["popUpContextMenu", menu]);
        }
        destroy() {
          calls.push(["destroy"]);
        }
      }
      const restore = () => calls.push(["mainShowOrRestore"]);
      const close = (...args) => calls.push(["mainClose", ...args]);
      const windowManager = {
        mainWindow: {
          showOrRestore: restore,
          toggleDevtools: () => {},
          close,
          repositionWindowIfInvisible: () => {},
        },
        auxWindow: {
          settings: { enabled: false },
          showOrRestore: () => {},
          toggleDevtools: () => {},
          repositionWindowIfInvisible: () => {},
        },
        opggWindow: {
          settings: { enabled: false },
          showOrRestore: () => {},
          toggleDevtools: () => {},
          repositionWindowIfInvisible: () => {},
        },
        ongoingGameWindow: {
          settings: { enabled: false },
          toggleDevtools: () => {},
          repositionWindowIfInvisible: () => {},
        },
        cdTimerWindow: {
          settings: { enabled: false },
          toggleDevtools: () => {},
          repositionWindowIfInvisible: () => {},
        },
      };
      const sandbox = {
        Tray,
        Menu,
        MenuItem,
        app: {},
        nativeImage: {},
        i18next: { t: (key) => key },
        AppCommonMain: { id: "app-common-main" },
        process: { platform: "win32" },
      };
      const build = vm.runInNewContext(
        `(function () {${extractMethod(source, "build()").replace(/!([.;),])/g, "$1")}})`,
        sandbox,
      );
      const destroy = vm.runInNewContext(
        `(function () {${extractMethod(source, "destroy()").replace(/!([.;),])/g, "$1")}})`,
        sandbox,
      );
      const destroyTrayMethod = vm.runInNewContext(
        `(function () {${extractMethod(source, "  destroyTrayIcon() {").replace(/!([.;),])/g, "$1")}})`,
        sandbox,
      );
      const controller = {
        context: {
          ipc: { sendEvent: () => {} },
          windowManager,
          appCommon: { settings: { locale: "en" } },
        },
        _context: {
          ipc: { sendEvent: () => {} },
          windowManager,
          appCommon: { settings: { locale: "en" } },
        },
        _trayIcon: null,
        _contextMenu: null,
        _createTrayIcon: () => "icon",
        destroyTrayIcon: () => destroyTrayMethod.call(controller),
        mainWindowDevTrayItem: null,
        auxWindowTrayItem: null,
        auxWindowDevTrayItem: null,
        opggWindowTrayItem: null,
        opggWindowDevTrayItem: null,
        ongoingGameWindowDevTrayItem: null,
        cdTimerWindowDevTrayItem: null,
        quitTrayItem: null,
        adjustAllWindowPositionsTrayItem: null,
      };

      build.call(controller);
      trayInstances[0].listeners.get("click")();
      menus[1][0].click();
      expect(
        calls.filter(([name]) => name === "mainShowOrRestore"),
      ).toHaveLength(2);
      menus[1].find((item) => item?.label === "tray.quit").click();
      expect(calls).toContainEqual(["mainClose", true]);

      destroy.call(controller);
      expect(calls).toContainEqual(["destroy"]);
      expect(calls).toContainEqual(["setApplicationMenu", null]);
      expect(controller._trayIcon).toBeNull();
    },
  );
});
