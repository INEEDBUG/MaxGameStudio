import "./index.css";
import { restoreLegacyElectronUiState } from "./utils/legacyElectronUiState";

async function configureDesktopSession() {
  if (!window.__TAURI_INTERNALS__) return;
  const [{ invoke }, { setDesktopSessionToken }] = await Promise.all([
    import("@tauri-apps/api/core"),
    import("./api/api.js"),
  ]);
  setDesktopSessionToken(await invoke("backend_session_token"));
}

function showBootFailure(error) {
  console.error("[Desktop] UI bootstrap failed", error);
  const root = document.getElementById("root");
  if (!root) return;
  root.innerHTML = `
    <main style="min-height:100vh;display:grid;place-items:center;background:#101114;color:#e4e4e7;padding:32px;font-family:Inter,system-ui,sans-serif">
      <section style="max-width:520px;border:1px solid rgba(248,113,113,.3);border-radius:16px;background:#17191d;padding:24px;box-shadow:0 24px 80px rgba(0,0,0,.45)">
        <strong style="color:#fca5a5">界面启动失败</strong>
        <p style="color:#a1a1aa;line-height:1.65">程序仍在运行，重新加载界面即可重试。</p>
        <button id="insight-reload-ui" style="border:0;border-radius:8px;background:#0ea5e9;color:white;padding:9px 15px;font-weight:700;cursor:pointer">重新加载界面</button>
      </section>
    </main>`;
  document.getElementById("insight-reload-ui")?.addEventListener("click", () => window.location.reload());
}

configureDesktopSession()
  .catch((error) => console.error("[Desktop Session] Token bootstrap failed", error))
  .then(() => restoreLegacyElectronUiState())
  .then(() => import("./renderApp.jsx"))
  .catch(showBootFailure);
