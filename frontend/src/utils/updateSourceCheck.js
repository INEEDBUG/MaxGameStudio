import { check } from "@tauri-apps/plugin-updater";

// Close a late native resource after a UI timeout. The native request itself
// also has a timeout; a Promise.race alone would leak pending checks.
export async function boundedUpdateCheck(timeout, checkFn = check) {
  let timer;
  let expired = false;
  const pending = Promise.resolve().then(() => checkFn({ timeout }));
  pending.then((result) => { if (expired) void result?.close().catch(() => {}); }, () => {});
  try {
    return await Promise.race([pending, new Promise((_, reject) => {
      timer = setTimeout(() => { expired = true; reject(new Error("检查更新超时 / Update check timed out")); }, timeout);
    })]);
  } finally { clearTimeout(timer); }
}
