import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  statSync,
} from "node:fs";
import { homedir } from "node:os";
import { dirname, isAbsolute, join, resolve } from "node:path";

export const LIBUNWIND_NAME = "libunwind.dll";

const GNU_HINT_KEYS = [
  "RUSTUP_TOOLCHAIN",
  "CARGO_BUILD_TARGET",
  "TARGET",
  "HOST",
  "RUSTC",
];

function isFile(path) {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function isDirectory(path) {
  try {
    return statSync(path).isDirectory();
  } catch {
    return false;
  }
}

export function isGnuBuild(env = process.env) {
  return GNU_HINT_KEYS.some((key) => /gnu/i.test(String(env[key] || "")));
}

export function resolveRustupToolchainRoot(env = process.env) {
  const configuredToolchain = String(env.RUSTUP_TOOLCHAIN || "").trim();
  if (configuredToolchain) {
    if (isAbsolute(configuredToolchain)) {
      const directRoot = resolve(configuredToolchain);
      if (isDirectory(directRoot)) return directRoot;
    }

    const rustupHome = String(env.RUSTUP_HOME || "").trim() || join(homedir(), ".rustup");
    const namedRoot = join(resolve(rustupHome), "toolchains", configuredToolchain);
    if (isDirectory(namedRoot)) return namedRoot;
  }

  const configuredRustc = String(env.RUSTC || "").trim();
  if (configuredRustc && isAbsolute(configuredRustc)) {
    const rustcPath = resolve(configuredRustc);
    const binDir = dirname(rustcPath);
    const toolchainRoot = dirname(binDir);
    if (isFile(rustcPath) && isDirectory(toolchainRoot)) return toolchainRoot;
  }

  return null;
}

export function resolveLibunwindSource(env = process.env) {
  const explicit = String(env.CS2_INSIGHT_LIBUNWIND_DLL || "").trim();
  if (explicit) {
    if (!isAbsolute(explicit)) {
      throw new Error("CS2_INSIGHT_LIBUNWIND_DLL must be an absolute path");
    }
    const explicitPath = resolve(explicit);
    if (!isFile(explicitPath)) {
      throw new Error(`Configured libunwind.dll was not found: ${explicitPath}`);
    }
    return explicitPath;
  }

  const toolchainRoot = resolveRustupToolchainRoot(env);
  const toolchainCandidate = toolchainRoot
    ? join(toolchainRoot, "bin", LIBUNWIND_NAME)
    : null;
  if (toolchainCandidate && isFile(toolchainCandidate)) return toolchainCandidate;

  const toolchain = String(env.RUSTUP_TOOLCHAIN || "").trim() || "the active Rust toolchain";
  throw new Error(
    `GNU Tauri build requires an x64 ${LIBUNWIND_NAME} from ${toolchain}. ` +
      "Set CS2_INSIGHT_LIBUNWIND_DLL to a controlled absolute path or install it in the active toolchain.",
  );
}

export function assertPeX64(filePath) {
  if (!isFile(filePath)) throw new Error(`Required PE file was not found: ${filePath}`);
  const data = readFileSync(filePath);
  if (data.length < 0x40 || data.readUInt16LE(0) !== 0x5a4d) {
    throw new Error(`Required PE file has no valid DOS header: ${filePath}`);
  }
  const peOffset = data.readUInt32LE(0x3c);
  if (
    peOffset + 6 > data.length ||
    data.readUInt32LE(peOffset) !== 0x00004550 ||
    data.readUInt16LE(peOffset + 4) !== 0x8664
  ) {
    throw new Error(`Required PE file is not x64: ${filePath}`);
  }
  return true;
}

export function stageLibunwind({ env = process.env, releaseRoot }) {
  const target = join(releaseRoot, LIBUNWIND_NAME);
  if (!isGnuBuild(env)) {
    if (existsSync(target)) rmSync(target, { force: true });
    return null;
  }

  const source = resolveLibunwindSource(env);
  assertPeX64(source);
  mkdirSync(releaseRoot, { recursive: true });
  copyFileSync(source, target);
  assertPeX64(target);
  return { source, target };
}
