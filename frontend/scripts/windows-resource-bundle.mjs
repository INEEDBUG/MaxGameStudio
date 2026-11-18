import { cpSync, existsSync, lstatSync, mkdtempSync, readdirSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";

function resourceFiles(root) {
  return readdirSync(root, { withFileTypes: true }).flatMap((entry) => {
    const path = join(root, entry.name);
    return entry.isDirectory() ? resourceFiles(path) : [path];
  });
}

// NSIS File cannot open long source paths even when Node and Rust can.
export function prepareWindowsResourceBundle(root, { platform = process.platform, tempRoot = tmpdir() } = {}) {
  if (platform !== "win32" || resourceFiles(root).every((path) => path.length < 240)) return null;
  const parent = realpathSync(resolve(tempRoot));
  const shortRoot = mkdtempSync(join(parent, "mgs-r-"));
  const cleanup = () => {
    if (!existsSync(shortRoot)) return;
    if (lstatSync(shortRoot).isSymbolicLink() || dirname(realpathSync(shortRoot)) !== parent) {
      throw new Error("Temporary NSIS resource directory ownership changed");
    }
    rmSync(shortRoot, { recursive: true, maxRetries: 5, retryDelay: 100 });
  };
  try {
    cpSync(root, shortRoot, { recursive: true });
    if (resourceFiles(shortRoot).some((path) => path.length >= 240)) {
      throw new Error("Windows TEMP is too long for NSIS resource staging");
    }
    return { root: shortRoot, cleanup };
  } catch (error) {
    cleanup();
    throw error;
  }
}
