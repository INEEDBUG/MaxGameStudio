import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import {
  assertPeX64,
  resolveLibunwindSource,
  resolveRustupToolchainRoot,
  stageLibunwind,
} from "./tauri-native-runtime.mjs";

const temporaryRoots = [];

function makePe(machine = 0x8664) {
  const data = Buffer.alloc(0x100);
  data.writeUInt16LE(0x5a4d, 0);
  data.writeUInt32LE(0x80, 0x3c);
  data.writeUInt32LE(0x00004550, 0x80);
  data.writeUInt16LE(machine, 0x84);
  return data;
}

function makeToolchain() {
  const root = mkdtempSync(join(tmpdir(), "mgs-native-runtime-"));
  temporaryRoots.push(root);
  const rustupHome = join(root, ".rustup");
  const toolchain = "stable-test-x86_64-pc-windows-gnullvm";
  const bin = join(rustupHome, "toolchains", toolchain, "bin");
  const runtime = join(bin, "libunwind.dll");
  const rustc = join(bin, "rustc.exe");
  mkdirSync(bin, { recursive: true });
  writeFileSync(runtime, makePe());
  writeFileSync(rustc, "rustc");
  return { root, rustupHome, toolchain, bin, runtime, rustc };
}

afterEach(() => {
  while (temporaryRoots.length) rmSync(temporaryRoots.pop(), { recursive: true, force: true });
});

describe("GNU Tauri native runtime staging", () => {
  test("resolves the x64 libunwind from the active named Rust toolchain", () => {
    const fixture = makeToolchain();
    const env = {
      RUSTUP_HOME: fixture.rustupHome,
      RUSTUP_TOOLCHAIN: fixture.toolchain,
    };

    expect(resolveRustupToolchainRoot(env)).toBe(join(fixture.rustupHome, "toolchains", fixture.toolchain));
    expect(resolveLibunwindSource(env)).toBe(fixture.runtime);
    expect(assertPeX64(fixture.runtime)).toBe(true);
  });

  test("accepts only an absolute controlled override and rejects non-x64 input", () => {
    const fixture = makeToolchain();
    expect(resolveLibunwindSource({ CS2_INSIGHT_LIBUNWIND_DLL: fixture.runtime })).toBe(fixture.runtime);
    expect(() => resolveLibunwindSource({ CS2_INSIGHT_LIBUNWIND_DLL: "libunwind.dll" })).toThrow(
      /absolute path/,
    );
    const arm = join(fixture.bin, "arm-libunwind.dll");
    writeFileSync(arm, makePe(0xaa64));
    expect(() => assertPeX64(arm)).toThrow(/not x64/);
  });

  test("stages GNU runtime and removes stale output for non-GNU builds", () => {
    const fixture = makeToolchain();
    const releaseRoot = join(fixture.root, "target", "release");
    const staged = stageLibunwind({
      env: { RUSTUP_HOME: fixture.rustupHome, RUSTUP_TOOLCHAIN: fixture.toolchain },
      releaseRoot,
    });
    expect(staged.source).toBe(fixture.runtime);
    expect(readFileSync(staged.target)).toEqual(readFileSync(fixture.runtime));

    const msvcTarget = join(releaseRoot, "libunwind.dll");
    expect(
      stageLibunwind({
        env: { RUSTUP_HOME: fixture.rustupHome, RUSTUP_TOOLCHAIN: "stable-x86_64-pc-windows-msvc" },
        releaseRoot,
      }),
    ).toBeNull();
    expect(() => readFileSync(msvcTarget)).toThrow();
  });
});
