import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, expect, test } from "vitest";
import { prepareWindowsResourceBundle } from "./windows-resource-bundle.mjs";

describe("Windows NSIS resource paths", () => {
  test("leaves normal and non-Windows resource roots in place", () => {
    expect(prepareWindowsResourceBundle("C:/short/resources", { platform: "linux" })).toBeNull();
    const root = mkdtempSync(join(tmpdir(), "mgs-resource-test-"));
    try {
      writeFileSync(join(root, "small.txt"), "unchanged");
      expect(prepareWindowsResourceBundle(root, { platform: "win32" })).toBeNull();
    } finally { rmSync(root, { recursive: true }); }
  });

  test("shortens deep sources without changing payload bytes or deleting adjacent files", () => {
    const parent = mkdtempSync(join(tmpdir(), "mgs-resource-test-"));
    const source = join(parent, "restored-project-".repeat(8), "bundle-resources");
    const relative = "python/Lib/site-packages/openai/types/beta/realtime/conversation_item_input_audio_transcription_completed_event.py";
    mkdirSync(dirname(join(source, relative)), { recursive: true });
    writeFileSync(join(source, relative), "fixture payload\n");
    writeFileSync(join(parent, "retain.txt"), "keep");
    let short;
    try {
      short = prepareWindowsResourceBundle(source, { platform: "win32", tempRoot: parent });
      expect(short).not.toBeNull();
      expect(join(short.root, relative).length).toBeLessThan(240);
      expect(readFileSync(join(short.root, relative), "utf8")).toBe("fixture payload\n");
      short.cleanup();
      expect(readFileSync(join(parent, "retain.txt"), "utf8")).toBe("keep");
      expect(readFileSync(join(source, relative), "utf8")).toBe("fixture payload\n");
    } finally { short?.cleanup(); rmSync(parent, { recursive: true }); }
  });
});
