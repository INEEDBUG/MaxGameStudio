export const DESKTOP_CLOSE_ACTIONS = Object.freeze(["ask", "tray", "exit"]);

export function normalizeDesktopCloseAction(action, legacyCloseToTray = true) {
  if (DESKTOP_CLOSE_ACTIONS.includes(action)) return action;
  return legacyCloseToTray === false ? "exit" : "ask";
}

export function desktopCloseActionPayload(action) {
  const normalized = normalizeDesktopCloseAction(action);
  return {
    close_action: normalized,
    close_to_tray: normalized !== "exit",
  };
}
