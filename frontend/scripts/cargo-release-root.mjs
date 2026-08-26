import { isAbsolute, join, resolve } from "node:path";

export function resolveCargoReleaseRoot({ frontendRoot, cargoTargetDir, cargoBuildTarget }) {
  const tauriRoot = join(frontendRoot, "src-tauri");
  if (!cargoTargetDir) return join(tauriRoot, "target", "release");
  const targetRoot = isAbsolute(cargoTargetDir)
    ? cargoTargetDir
    : resolve(tauriRoot, cargoTargetDir);
  // Tauri's CLI already appends an explicit `--target` to its bundle path,
  // but setting CARGO_BUILD_TARGET through Cargo's environment does not alter
  // the bundle resolver's expected profile directory.  The wrapper invokes
  // Tauri without `--target`, so artifacts are always validated beneath the
  // selected Cargo target directory's profile root.
  void cargoBuildTarget;
  return join(targetRoot, "release");
}
