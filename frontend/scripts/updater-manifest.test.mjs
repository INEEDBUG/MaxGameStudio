import { describe, expect, test } from "vitest";
import {
  createGithubUpdaterManifest,
  githubReleaseAssetName,
} from "./updater-manifest.mjs";

describe("GitHub updater manifest", () => {
  test("matches GitHub's normalized release asset name", () => {
    expect(githubReleaseAssetName("MaxGameStudio_2.5.9_x64-setup.exe"))
      .toBe("MaxGameStudio_2.5.9_x64-setup.exe");
  });

  test("creates a signed Windows static manifest", () => {
    const manifest = createGithubUpdaterManifest({
      version: "2.5.9",
      repository: "INEEDBUG/MaxGameStudio",
      installerName: "MaxGameStudio_2.5.9_x64-setup.exe",
      signature: "signed-payload",
      notes: "更新说明",
      pubDate: "2026-08-13T00:00:00.000Z",
      userReleaseNotes: {
        version: "2.5.9",
        fixed: ["修复更新失败"],
        added: ["新增更新说明"],
        optimized: [],
      },
    });

    expect(manifest.version).toBe("2.5.9");
    expect(manifest.update_mode).toBe("normal");
    expect(manifest.platforms["windows-x86_64"]).toEqual({
      signature: "signed-payload",
      url: "https://github.com/INEEDBUG/MaxGameStudio/releases/download/v2.5.9/MaxGameStudio_2.5.9_x64-setup.exe",
    });
    expect(manifest.user_release_notes).toEqual({
      version: "2.5.9",
      fixed: ["修复更新失败"],
      added: ["新增更新说明"],
      optimized: [],
    });
  });

  test("rejects prerelease versions from the formal updater manifest", () => {
    expect(() =>
      createGithubUpdaterManifest({
        version: "2.5.16-rc.1",
        repository: "INEEDBUG/MaxGameStudio",
        installerName: "MaxGameStudio_2.5.16-rc.1_x64-setup.exe",
        signature: "signed-payload",
      }),
    ).toThrow(/stable semantic version/);
  });

  test("rejects user notes for a different release", () => {
    expect(() =>
      createGithubUpdaterManifest({
        version: "3.0.1",
        repository: "INEEDBUG/MaxGameStudio",
        installerName: "MaxGameStudio_3.0.1_x64-setup.exe",
        signature: "signed-payload",
        userReleaseNotes: { version: "3.0.0", fixed: [], added: [], optimized: ["更快"] },
      }),
    ).toThrow(/version mismatch/);
  });
});
