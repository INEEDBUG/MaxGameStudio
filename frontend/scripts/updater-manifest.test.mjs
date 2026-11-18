// @vitest-environment node
import { describe, expect, test } from "vitest";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
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

  test("selects an explicit main beta release with an exact product tag and filename", () => {
    const fields = {
      channel: "beta",
      version: "3.2.0-beta.1",
      tag: "v3.2.0-beta.1",
      repository: "INEEDBUG/MaxGameStudio",
      installerName: "MaxGameStudio_3.2.0-beta.1_x64-setup.exe",
      signature: "signed-payload",
    };
    const manifest = createGithubUpdaterManifest(fields);
    expect(manifest).toMatchObject({ product: "maxgamestudio", channel: "beta", tag_name: fields.tag });
    expect(() => createGithubUpdaterManifest({ ...fields, tag: "league-v3.2.0-beta.1" })).toThrow(/tag/);
    expect(() => createGithubUpdaterManifest({ ...fields, installerName: "MaxGameStudioLeague_3.2.0-beta.1_x64-setup.exe" })).toThrow(/installer/);
  });

  test("does not sign up the superseded 3.1.4 installer", () => {
    const fields = { version: "3.1.4", repository: "INEEDBUG/MaxGameStudio", signature: "signed-payload" };
    expect(() => createGithubUpdaterManifest({ ...fields, installerName: "MaxGameStudio_3.1.4_x64-setup.exe" })).toThrow(/installer/);
    const manifest = createGithubUpdaterManifest({ ...fields, installerName: "MaxGameStudio_3.1.4_x64-repair-setup.exe" });
    expect(manifest.platforms["windows-x86_64"].url).toContain("_x64-repair-setup.exe");
  });

  test("writes only the beta manifest and ignores unrelated installers in the bundle directory", () => {
    const targetDir = mkdtempSync(join(tmpdir(), "mgs-updater-test-"));
    const nsisDir = join(targetDir, "release", "bundle", "nsis");
    const version = "3.2.0-beta.1";
    const installer = `MaxGameStudio_${version}_x64-setup.exe`;
    const userNotesPath = join(targetDir, "notes.json");
    try {
      mkdirSync(nsisDir, { recursive: true });
      writeFileSync(join(nsisDir, installer), "");
      writeFileSync(join(nsisDir, `${installer}.sig`), "main-signature");
      writeFileSync(join(nsisDir, "MaxGameStudio_9.0.0_x64-setup.exe"), "");
      writeFileSync(join(nsisDir, "MaxGameStudioLeague_99.0.0_x64-setup.exe"), "");
      writeFileSync(userNotesPath, JSON.stringify({ version, fixed: ["A fix"], added: [], optimized: [] }));
      const result = spawnSync(process.execPath, [fileURLToPath(new URL("./generate-github-updater-manifest.mjs", import.meta.url))], {
        encoding: "utf8",
        env: {
          ...process.env,
          BUILD_VERSION: version,
          UPDATE_CHANNEL: "beta",
          GITHUB_REF_TYPE: "tag",
          GITHUB_REF_NAME: `v${version}`,
          GITHUB_REPOSITORY: "INEEDBUG/MaxGameStudio",
          USER_RELEASE_NOTES_PATH: userNotesPath,
          CARGO_TARGET_DIR: targetDir,
        },
      });
      expect(result.status, result.stderr).toBe(0);
      const manifest = JSON.parse(readFileSync(join(nsisDir, "beta.json"), "utf8"));
      expect(manifest.platforms["windows-x86_64"]).toEqual({
        signature: "main-signature",
        url: `https://github.com/INEEDBUG/MaxGameStudio/releases/download/v${version}/${installer}`,
      });
      expect(existsSync(join(nsisDir, "latest.json"))).toBe(false);
    } finally {
      rmSync(targetDir, { recursive: true, force: true });
    }
  });
});
