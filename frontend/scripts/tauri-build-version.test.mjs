import { describe, expect, test } from "vitest";
import { join } from "node:path";
import { resolveCargoReleaseRoot } from "./cargo-release-root.mjs";

describe("Tauri release artifact resolver", () => {
  test("uses the workspace target directory by default", () => {
    const frontendRoot = join("D:\\workspace", "frontend");
    expect(resolveCargoReleaseRoot({ frontendRoot })).toBe(
      join(frontendRoot, "src-tauri", "target", "release"),
    );
  });

  test("honors an absolute custom Cargo target directory", () => {
    expect(resolveCargoReleaseRoot({
      frontendRoot: "D:\\workspace\\frontend",
      cargoTargetDir: "D:\\cargo-target\\candidate",
    })).toBe(join("D:\\cargo-target\\candidate", "release"));
  });

  test("does not append Cargo's environment default target twice", () => {
    expect(resolveCargoReleaseRoot({
      frontendRoot: "D:\\workspace\\frontend",
      cargoTargetDir: "D:\\cargo-target\\candidate",
      cargoBuildTarget: "x86_64-pc-windows-gnullvm",
    })).toBe(join("D:\\cargo-target\\candidate", "release"));
  });
});
