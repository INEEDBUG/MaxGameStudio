# Desktop storage and administrator launch

The 3.1.2 Windows installer was verified locally before publishing.

## Locations

On Windows the host asks for a dedicated storage directory before creating a
WebView or starting the Python backend. The suggested location is a per-user
directory on an available fixed non-system volume. Choosing the system volume
requires explicit confirmation. An unavailable selected location is an error,
not permission to fall back to `%APPDATA%` or system `%TEMP%`.

The selected root contains:

- `data`: backend configuration, databases and app-managed analysis data/logs.
- `webview`: Tauri WebView2 profile/cache shared by the app's windows.
- `league-runtime`: the ordinary-permission League profile.
- `temp`: process-local temporary files, updater staging, administrator launch scripts.
- `window-state.json`: window position/size persistence.

Only the small locator lives in `HKCU\Software\MaxGameStudio\Storage`, value
`LocationV1`. Settings → Paths → Unified app storage shows the location, usage,
pending changes and the separately protected administrator directory.

Administrator League sessions and profiles remain under
`<selected volume>:\MaxGameStudioAdminRuntime`, protected for Administrators and
SYSTEM. They must not inherit ordinary-user write access. A one-use launcher is
created in the writable selected `temp/admin-launchers` directory **before** UAC;
its verified bytes and ancestor directory handles stay locked until the elevated
supervisor exits. The host itself remains unelevated. Never solve a launch error
by weakening those ACLs or elevating the entire host.

## Migration and recovery

A change made in Settings is scheduled for the next start and can be cancelled.
Close all workspaces and the host first. Migration refuses a live backend or
another host/workspace, nested roots, a nonempty target, or reparse-point paths.
The only legacy exception is a direct `data/cache` or `data/trash` directory
junction whose real target is on the selected local non-system volume. Its
target must contain no further links; migration copies it as an ordinary
directory, rechecks the alias before promotion and leaves the old link intact.
It copies to a restricted staging directory on the selected volume, verifies
content hashes and rechecks source/process state before promoting the directory.
The registry location switches only after success. Originals are not deleted.

On initial migration, recognized legacy configuration/database payload takes
precedence over empty or metadata-only directories. Older Electron profiles are
preserved offline; no old executable is launched to export preferences. An
existing exported UI-state file is copied, but unexported ancient Electron
preferences may need manual recovery. The `.storage-migration-v1.json` marker
records source directories and a copy manifest. It is local recovery metadata,
not a release artifact or upload payload.

If a scheduled copy fails, the native dialog can cancel it and continue with the
original root. After a successful switch, do not overwrite newer files with an
old snapshot. Close all processes and preserve both versions before any manual
recovery. Keep source backups until the new location passes real usage checks;
copying alone does not reclaim the originals' disk space.

The protected League profile has a separate privileged copy path: when moving
the data volume with the installation path unchanged, it copies and verifies
the old protected profile and retains the original. Moving the installation path
at the same time changes its identity hash; automatic recovery from an unrelated
old protected profile is deliberately not guessed.

## Boundaries

- Fresh interactive installations prefer a fixed non-system volume. Existing
  installations and explicit `/D=` choices are respected. A fresh silent install
  must specify `/D=`; upgrades preserve their registered directory.
- Windows-owned registry, crash handling, installer/WebView runtime components
  and an empty framework config directory can still have system-drive footprints.
  This is not a promise of zero Windows C-drive writes.
- Game-owned CFG files, user-selected demo/output locations and external tools
  are not relocated by changing the app storage root.
- Protected administrator storage is shown separately; its contents are not
  included in the ordinary-user directory-size total.
- Changes to `TEMP`/`TMP` apply to app processes only. Python pins `tempfile` to
  the explicit desktop directory so a missing volume cannot trigger fallback.

## Verification

Use temporary fixtures for migration and failure injection; never point test
fixtures at a user's real game CFG or profile. Run migration/temp/installer
pytest tests, frontend storage UI tests, full frontend build, Rust unit tests and
Clippy. Compile the actual NSIS installer. Finally verify ordinary-user startup,
profile migration, selected-volume file writes, UAC cancellation/success,
protected-runtime launch and return to the host on the local machine.

For 3.1.2, installed-process checks verified the actual WebView profile path on
the chosen non-system volume, an unelevated host with an elevated League child,
ordinary parallel mode, and backend/WebView teardown and restoration in memory
mode. The current Tauri WindowConfig conversion drops the data-directory field;
startup windows therefore use the explicit native builder setter, just like
restored windows. Configuration-only assertions do not replace this process test.
Interactive UAC cancellation was not re-exercised on the acceptance machine
because its existing Windows policy automatically approved elevation; the test
did not change that policy.
