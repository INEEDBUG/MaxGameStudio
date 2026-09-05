# Desktop storage and administrator launch

This describes the current local update; publication waits for local acceptance.

## Locations

Startup does not copy or migrate existing user data. A previously selected
directory is reused; an already-used non-system directory can be recovered when
its locator is missing. Otherwise recognized legacy data is used in place.
Fresh profiles use a dedicated directory on an available fixed non-system volume
where possible. Choosing the system volume manually requires confirmation.
An unavailable selected location is an error, not permission to create an empty
replacement profile or fall back to system `%TEMP%`.

The selected root contains:

- `data`: backend configuration, databases and app-managed analysis data/logs.
- `webview`: Tauri WebView2 profile/cache shared by the app's windows.
- `league-runtime`: the ordinary-permission League profile.
- `temp`: process-local temporary files, updater staging, administrator launch scripts.
- `window-state.json`: window position/size persistence.

Only the small locator lives in `HKCU\Software\MaxGameStudio\Storage`, value
`LocationV1`. Settings → Paths → Data storage location shows the actual data,
logs, cache, WebView, League profile and temporary paths, pending changes and
the separately protected administrator directory. Legacy components may remain
in different directories; the displayed paths are authoritative.

Administrator League sessions and profiles remain under
`<selected volume>:\MaxGameStudioAdminRuntime`, protected for Administrators and
SYSTEM. They must not inherit ordinary-user write access. A one-use launcher is
created in the writable selected `temp/admin-launchers` directory **before** UAC;
its verified bytes and ancestor directory handles stay locked until the elevated
supervisor exits. The host itself remains unelevated. Never solve a launch error
by weakening those ACLs or elevating the entire host.

## Manual location changes and recovery

A change made in Settings is scheduled for the next start and can be cancelled.
It switches the locator only: no copying, moving or deleting ordinary user data.
An empty directory starts with new settings; a recognized existing data directory
uses its own settings. Unknown nonempty directories are rejected. The old location
is retained and can be selected again, including its original legacy component paths.

If a requested target becomes unavailable or unsafe before restart, the change
is cancelled and the original location is revalidated and reused. Settings shows
the failure reason and lets the user select a location again. No data is copied
or deleted. If the original location or a known component is also unavailable,
startup still fails closed instead of creating an empty replacement profile.

Old queued migration requests are cancelled, never reinterpreted as permission
to switch to an empty profile. An existing `.storage-migration-v1.json` marker
can identify previously migrated data, but normal runtime writes do not trigger
recopying or invalidate that data. Known components that disappear fail closed.
Keep both locations when recovering manually; switching does not merge settings
or reclaim disk space. The old copy utility remains for historical tests, but
startup no longer invokes it.

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

Use temporary fixtures for location selection and failure injection; never point test
fixtures at a user's real game CFG or profile. Run migration/temp/installer
pytest tests, frontend storage UI tests, full frontend build, Rust unit tests and
Clippy. Compile the actual NSIS installer. Finally verify ordinary-user startup,
in-place profile reuse, selected-volume file writes, UAC cancellation/success,
protected-runtime launch and return to the host on the local machine.

Earlier 3.1.2 installed-process checks verified the actual WebView profile path on
the chosen non-system volume, an unelevated host with an elevated League child,
ordinary parallel mode, and backend/WebView teardown and restoration in memory
mode. The current Tauri WindowConfig conversion drops the data-directory field;
startup windows therefore use the explicit native builder setter, just like
restored windows. Configuration-only assertions do not replace this process test.
Interactive UAC cancellation was not re-exercised on the acceptance machine
because its existing Windows policy automatically approved elevation; the test
did not change that policy. These earlier checks do not replace acceptance of
the current switch-only storage and prewarmed handoff update.
