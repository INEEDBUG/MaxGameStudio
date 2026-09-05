// @vitest-environment node
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { EventEmitter } from "node:events";
import { transformWithEsbuild } from "vite";

const helper = fs.readFileSync(path.resolve(__dirname, "../../../packaging/windows/stage-league-prewarm.ps1"), "utf8");
const source = helper.split("$module = @'")[1].split("'@")[0];
const compiled = await transformWithEsbuild(source, "mgs-prewarm.ts", { loader: "ts", format: "cjs" });

function gate({ warm = true, session = "0123456789abcdef0123456789abcdef", profile = "C:\\MaxGameStudio\\profile", activateInitially = false } = {}) {
  const watcher = new EventEmitter();
  watcher.close = vi.fn();
  const files = new Set();
  let activateExists = activateInitially;
  let watchCallback;
  const deferred = [];
  const module = { exports: {} };
  vm.runInNewContext(compiled.code, {
    module, exports: module.exports,
    process: { argv: warm ? ["--maxgamestudio-prewarm", `--maxgamestudio-prewarm-session=${session}`, `--user-data-dir=${profile}`] : [] },
    require: (name) => {
      if (name === "node:fs") return {
        watch: vi.fn((_profile, callback) => { watchCallback = callback; return watcher; }),
        existsSync: (file) => files.has(file) || (activateExists && file.endsWith(".activate")),
        writeFileSync: (file, _value, options) => {
          if (options?.flag === "wx" && files.has(file)) {
            const error = new Error("exists");
            error.code = "EEXIST";
            throw error;
          }
          files.add(file);
        },
      };
      if (name === "node:path") return { isAbsolute: (value) => /^[A-Za-z]:[\\/]/.test(value) || value.startsWith("/"), win32: { isAbsolute: (value) => /^[A-Za-z]:[\\/]/.test(value) || value.startsWith("\\\\") }, join: (...parts) => parts.join("\\") };
      throw new Error(name);
    },
    setImmediate: (callback) => deferred.push(callback),
  });
  return { ...module.exports, watcher, files, deferred, profile, session, setActivateFile: () => { activateExists = true; }, trigger: (...args) => watchCallback(...args) };
}

describe("embedded prewarm activation gate", () => {
  it("does not run providers or display the window until explicit activation", () => {
    const g = gate();
    const actions = [];
    g.onActivated(() => actions.push("provider"));
    g.onActivated(() => { actions.push("window"); g.reportShown(); }, 100);
    g.reportReady();
    expect(g.isPrewarming()).toBe(true);
    expect(actions).toEqual([]);
    expect([...g.files]).toEqual(["C:\\MaxGameStudio\\profile\\prewarm-0123456789abcdef0123456789abcdef.ready"]);
    g.trigger("change", "prewarm-0123456789abcdef0123456789abcdef.activate");
    expect(actions).toEqual([]);
    g.setActivateFile();
    g.trigger("change", "prewarm-ffffffffffffffffffffffffffffffff.activate");
    expect(actions).toEqual([]);
    g.trigger("change", "prewarm-0123456789abcdef0123456789abcdef.activate");
    expect(actions).toEqual(["window"]);
    expect([...g.files]).toContain("C:\\MaxGameStudio\\profile\\prewarm-0123456789abcdef0123456789abcdef.shown");
    g.deferred.forEach((callback) => callback());
    expect(actions).toEqual(["window", "provider"]);
    g.trigger("change", "prewarm-0123456789abcdef0123456789abcdef.activate");
    expect(g.deferred).toHaveLength(1);
    expect(g.watcher.close).toHaveBeenCalledOnce();
  });

  it("handles activation before renderer ready and emits each acknowledgement once", () => {
    const g = gate({ activateInitially: true });
    const actions = [];
    g.onActivated(() => actions.push("provider"));
    g.onActivated(() => { actions.push("window"); g.reportShown(); }, 100);
    expect(g.isPrewarming()).toBe(true);
    expect(actions).toEqual([]);
    expect(g.watcher.close).toHaveBeenCalledOnce();
    g.reportReady();
    expect(g.isPrewarming()).toBe(false);
    expect(actions).toEqual(["window"]);
    g.deferred.forEach((callback) => callback());
    expect(actions).toEqual(["window", "provider"]);
    g.reportReady();
    g.reportShown();
    expect([...g.files]).toEqual(expect.arrayContaining([
      "C:\\MaxGameStudio\\profile\\prewarm-0123456789abcdef0123456789abcdef.ready",
      "C:\\MaxGameStudio\\profile\\prewarm-0123456789abcdef0123456789abcdef.shown",
    ]));
  });

  it("rejects missing or malformed sessions and preserves the normal startup path", () => {
    expect(() => gate({ session: "bad" })).toThrow("Invalid MaxGameStudio prewarm session");
    expect(() => gate({ session: "" })).toThrow("Invalid MaxGameStudio prewarm session");
    expect(() => gate({ profile: "relative-profile" })).toThrow("Invalid MaxGameStudio prewarm session");
    const normal = gate({ warm: false });
    const callback = vi.fn();
    normal.onActivated(callback);
    normal.reportReady();
    normal.reportShown();
    expect(callback).toHaveBeenCalledOnce();
    expect(normal.files).toEqual(new Set());
  });

  it("does not write marker files during normal startup", () => {
    const normal = gate({ warm: false });
    normal.reportReady();
    normal.reportShown();
    expect(normal.files).toEqual(new Set());
  });

  it("stages guards at client, game, automation and visible-window boundaries", () => {
    for (const name of ["league-client", "riot-client", "game-client", "league-client-ux", "auto-select", "auto-gameflow", "auto-champ-config", "auto-misc", "ongoing-game", "respawn-timer", "sgp"]) {
      expect(helper).toContain(`shards\\${name}\\index.ts`);
    }
    expect(helper).toContain("show: isPrewarming() ? false : rest.show");
    expect(helper).toContain("this._namespaceSuffix === 'main-window'");
    expect(helper).toContain("$fallbackReadyAnchor");
    expect(helper).toContain("this._namespaceSuffix === 'main-window') reportReady()");
    expect(helper).toContain("this.window?.isVisible()");
    expect(helper).toContain("maxgamestudio-prewarm-session=");
    expect(helper).toContain("fs.watch(profile");
    expect(helper).toContain("existsSync(path.join(profile, activateFileName))");
    expect(helper).toContain("if (isPrewarming()) return");
    expect(helper).toContain("_processNativeKeyEvent(key)");
    expect(helper).not.toContain("process.stdin");
    expect(helper).not.toContain("createServer(");
  });
});
