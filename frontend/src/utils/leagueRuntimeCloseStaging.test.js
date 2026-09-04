import fs from "node:fs";
import path from "node:path";

const stagingScriptPath = path.resolve(__dirname, "../../../packaging/windows/stage-league-runtime.ps1");
const stagingScript = fs.readFileSync(stagingScriptPath, "utf8");

describe("embedded League runtime close staging contract", () => {
  it("does not bypass the close confirmation and minimizes the BrowserWindow in embedded mode", () => {
    expect(stagingScript).not.toContain("$embeddedCloseGuard");
    expect(stagingScript).toContain("$embeddedCloseMinimize");
    expect(stagingScript).toContain("this._window?.minimize()");
    expect(stagingScript).toContain("event.preventDefault()");
    expect(stagingScript).toContain("process.env.MAXGAMESTUDIO_EMBEDDED === '1'");
    expect(stagingScript).toContain("$closeStrategyAnchor");
    expect(stagingScript).toContain("$patchedCloseStrategyAnchor = $closeStrategyAnchor + $windowLineBreak + $embeddedCloseMinimize");
    expect(stagingScript).toContain("$trueCloseIndex");
    expect(stagingScript).toContain("this._trueClose || this._context.shared.global.isReadyToQuit");
  });

  it("keeps the two close choices branded for both supported locales", () => {
    expect(stagingScript).toContain("Close League Workspace");
    expect(stagingScript).toContain("Minimize League Workspace");
    expect(stagingScript).toContain("Return to MaxGameStudio");
    expect(stagingScript).toContain("$zhCloseLeagueWorkspace");
    expect(stagingScript).toContain("$zhMinimizeLeagueWorkspace");
    expect(stagingScript).toContain("$zhReturnToMaxGameStudio");
    expect(stagingScript).toContain("$zhQuitCommonAppName = (& $makeUnicodeText @(0x9000, 0x51FA)) + ' $t(common:appName)'");
  });

  it("keeps the staging script ASCII-only and constructs Chinese labels at build time", () => {
    expect([...stagingScript].filter((character) => character.charCodeAt(0) > 0x7f)).toEqual([]);
    expect(stagingScript).toContain("$makeUnicodeText");
  });
});
