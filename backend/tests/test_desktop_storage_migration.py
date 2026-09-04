import json
import os
import stat
import shutil
import subprocess
import tempfile
import types
from pathlib import Path

import pytest

from app.desktop_storage_migration import (
    MARKER_NAME,
    StorageMigrationError,
    migrate_storage,
)


def _tree(root: Path) -> None:
    (root / "data").mkdir(parents=True)
    (root / "webview" / "Local Storage").mkdir(parents=True)
    (root / "league-runtime").mkdir()
    (root / "data" / "db.sqlite-wal").write_bytes(b"\x00\xff\r\nraw")
    (root / "webview" / "Local Storage" / "state.bin").write_bytes(bytes(range(256)))
    (root / "league-runtime" / "runtime.txt").write_text("runtime", encoding="utf-8")


def _junction_fixture(tmp_path: Path, name: str) -> tuple[Path, Path, Path]:
    """Create a real junction only in an isolated test tree."""
    volume = tmp_path
    if os.name != "nt" or volume.drive.lower() == os.environ.get("SystemDrive", "C:").lower():
        pytest.skip("run with TEMP on a non-system volume for native junction fixtures")
    root = Path(tempfile.mkdtemp(prefix=f"storage-junction-{name}-", dir=volume))
    source = root / "source"
    target = root / "target"
    link = source / "data" / name
    target.mkdir(parents=True)
    source.joinpath("data").mkdir(parents=True)
    (target / "payload.bin").write_bytes(b"junction-payload")
    completed = subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/cmd.exe"), "/c", "mklink", "/J", str(link), str(target)],
                               capture_output=True, text=True, shell=False)
    if completed.returncode != 0:
        shutil.rmtree(root, ignore_errors=True)
        pytest.skip(f"junction creation unavailable: {completed.stderr}")
    return root, source, target


def _remove_junction(path: Path) -> None:
    if path.exists():
        assert path.lstat().st_reparse_tag == 0xA0000003
        path.rmdir()  # removes only the junction, never its target


def _make_junction(link: Path, target: Path) -> None:
    link.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(Path(os.environ["SystemRoot"]) / "System32/cmd.exe"), "/c", "mklink", "/J", str(link), str(target)],
                   capture_output=True, text=True, shell=False, check=True)


def test_copy_is_exact_and_sources_are_retained(tmp_path: Path):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    result = migrate_storage(source=source, destination=destination, process_check=False)
    assert result.mode == "migrated"
    assert (destination / "data" / "db.sqlite-wal").read_bytes() == b"\x00\xff\r\nraw"
    assert (source / "data" / "db.sqlite-wal").exists()
    marker = json.loads((destination / MARKER_NAME).read_text())
    assert marker["rollback_sources"] == [str(source)]
    assert migrate_storage(source=source, destination=destination, process_check=False).mode == "existing"


def test_destination_must_not_be_nonempty(tmp_path: Path):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    destination.mkdir()
    (destination / "keep").write_bytes(b"keep")
    with pytest.raises(StorageMigrationError, match="absent or empty"):
        migrate_storage(source=source, destination=destination, process_check=False)


def test_nested_paths_and_reparse_points_fail_closed(tmp_path: Path, monkeypatch):
    source = tmp_path / "source"
    _tree(source)
    with pytest.raises(StorageMigrationError, match="distinct"):
        migrate_storage(source=source, destination=source / "child", process_check=False)
    link = source / "data" / "link"
    link.write_bytes(b"fixture")
    original = __import__("app.desktop_storage_migration", fromlist=["_is_reparse"])._is_reparse
    monkeypatch.setattr(
        "app.desktop_storage_migration._is_reparse",
        lambda path: path == link or original(path),
    )
    with pytest.raises(StorageMigrationError, match="symlinks"):
        migrate_storage(source=source, destination=tmp_path / "out", process_check=False)


def test_copy_failure_cleans_only_its_staging(tmp_path: Path):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    def fail(_source: Path, _destination: Path) -> None:
        raise OSError("injected copy failure")
    with pytest.raises(OSError, match="injected"):
        migrate_storage(source=source, destination=destination, process_check=False, copy_impl=fail)
    assert not destination.exists()
    assert (source / "data" / "db.sqlite-wal").exists()
    assert not list(tmp_path.glob(".storage-migration-*"))


def test_legacy_layout_and_optional_window_state(tmp_path: Path):
    appdata, local, destination = tmp_path / "appdata", tmp_path / "local", tmp_path / "new"
    (appdata / "CS2 Insight Agent" / "data").mkdir(parents=True)
    (appdata / "MaxGameStudio" / "league-runtime").mkdir(parents=True)
    (local / "com.cs2insightagent.app").mkdir(parents=True)
    (appdata / "com.cs2insightagent.app").mkdir(parents=True)
    (appdata / "CS2 Insight Agent" / "data" / "config.bin").write_bytes(b"cfg")
    (appdata / "MaxGameStudio" / "league-runtime" / "x").write_bytes(b"x")
    (local / "com.cs2insightagent.app" / "webview.bin").write_bytes(b"wv")
    (appdata / "com.cs2insightagent.app" / ".window-state.json").write_bytes(b"{\r\n}")
    migrate_storage(legacy_appdata=appdata, legacy_localappdata=local, destination=destination, process_check=False)
    assert (destination / "window-state.json").read_bytes() == b"{\r\n}"


def test_empty_canonical_data_does_not_mask_populated_legacy(tmp_path: Path):
    appdata, local, destination = tmp_path / "appdata", tmp_path / "local", tmp_path / "new"
    (appdata / "CS2 Insight Agent" / "data").mkdir(parents=True)
    (appdata / "cs2-insight-agent" / "data").mkdir(parents=True)
    (appdata / "cs2-insight-agent" / "data" / "config.bin").write_bytes(b"legacy")
    migrate_storage(legacy_appdata=appdata, legacy_localappdata=local,
                    destination=destination, process_check=False)
    assert (destination / "data" / "config.bin").read_bytes() == b"legacy"


def test_metadata_only_canonical_does_not_mask_real_legacy_database(tmp_path: Path):
    appdata, local, destination = tmp_path / "appdata", tmp_path / "local", tmp_path / "new"
    canonical = appdata / "CS2 Insight Agent" / "data"
    legacy = appdata / "com.cs2insightagent.app" / "data"
    canonical.mkdir(parents=True)
    legacy.mkdir(parents=True)
    (canonical / "migration.log").write_bytes(b"metadata")
    (legacy / "cs2-insight.db").write_bytes(b"database")
    migrate_storage(legacy_appdata=appdata, legacy_localappdata=local, destination=destination, process_check=False)
    assert (destination / "data" / "cs2-insight.db").read_bytes() == b"database"
    assert (canonical / "migration.log").exists()


@__import__("pytest").mark.skipif(__import__("os").name != "nt", reason="Windows ACL")
def test_fresh_storage_acl_is_protected_on_windows(tmp_path: Path):
    import os
    import subprocess
    destination = tmp_path / "new"
    migrate_storage(legacy_appdata=tmp_path / "roaming", legacy_localappdata=tmp_path / "local",
                    destination=destination, process_check=False)
    env = dict(os.environ, MGS_ACL_TEST_PATH=str(destination))
    powershell = Path(os.environ["SystemRoot"]) / "System32/WindowsPowerShell/v1.0/powershell.exe"
    result = subprocess.run([str(powershell), "-NoProfile", "-Command",
        "[IO.Directory]::GetAccessControl($env:MGS_ACL_TEST_PATH).AreAccessRulesProtected"],
        capture_output=True, text=True, env=env, creationflags=subprocess.CREATE_NO_WINDOW)
    assert result.returncode == 0, result.stderr[:200]
    assert result.stdout.strip() == "True"


def test_empty_legacy_install_initializes_layout(tmp_path: Path):
    destination = tmp_path / "new"
    result = migrate_storage(legacy_appdata=tmp_path / "appdata", legacy_localappdata=tmp_path / "local",
                             destination=destination, process_check=False)
    assert result.file_count == 0
    assert all((destination / name).is_dir() for name in ("data", "webview", "league-runtime"))


def test_source_root_preserves_unknown_files_and_dirs(tmp_path: Path):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    (source / "unknown" / "nested").mkdir(parents=True)
    (source / "unknown" / "nested" / "raw.tmp").write_bytes(b"\xff\x00")
    (source / MARKER_NAME).write_text("interrupted", encoding="utf-8")
    migrate_storage(source=source, destination=destination, process_check=False)
    assert (destination / "unknown" / "nested" / "raw.tmp").read_bytes() == b"\xff\x00"
    assert not (destination / MARKER_NAME).read_text(encoding="utf-8").startswith("interrupted")


def test_marker_is_bound_to_exact_source_invocation(tmp_path: Path):
    source_a, source_b, destination = tmp_path / "a", tmp_path / "b", tmp_path / "out"
    _tree(source_a)
    _tree(source_b)
    migrate_storage(source=source_a, destination=destination, process_check=False)
    with pytest.raises(StorageMigrationError, match="absent or empty"):
        migrate_storage(source=source_b, destination=destination, process_check=False)


def test_source_mutation_and_corrupt_destination_are_rejected(tmp_path: Path):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    def mutate(src: Path, dst: Path) -> None:
        __import__("shutil").copytree(src, dst, dirs_exist_ok=True)
        (src / "data" / "db.sqlite-wal").write_bytes(b"changed")
    with pytest.raises(StorageMigrationError, match="hash mismatch|changed"):
        migrate_storage(source=source, destination=destination, process_check=False, copy_impl=mutate)
    def corrupt(src: Path, dst: Path) -> None:
        __import__("shutil").copytree(src, dst, dirs_exist_ok=True)
        (dst / "data" / "db.sqlite-wal").write_bytes(b"corrupt")
    with pytest.raises(StorageMigrationError, match="hash mismatch"):
        migrate_storage(source=source, destination=tmp_path / "out", process_check=False, copy_impl=corrupt)


def test_insufficient_space_and_replace_failure_clean_staging(tmp_path: Path, monkeypatch):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    monkeypatch.setattr("app.desktop_storage_migration.shutil.disk_usage",
                        lambda _path: type("Usage", (), {"free": 0})())
    with pytest.raises(StorageMigrationError, match="insufficient"):
        migrate_storage(source=source, destination=destination, process_check=False)
    monkeypatch.undo()
    real_replace = __import__("os").replace
    def fail_replace(src, dst):
        if str(src).find(".storage-migration-") >= 0:
            raise OSError("replace injected")
        return real_replace(src, dst)
    monkeypatch.setattr("app.desktop_storage_migration.os.replace", fail_replace)
    with pytest.raises(OSError, match="replace injected"):
        migrate_storage(source=source, destination=destination, process_check=False)
    assert not destination.exists()
    assert not list(tmp_path.glob(".storage-migration-*"))


def test_process_gate_allows_only_host_pid(tmp_path: Path, monkeypatch):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    monkeypatch.setattr("app.desktop_storage_migration.ensure_backend_stopped", lambda: None)
    monkeypatch.setattr("app.desktop_storage_migration._process_snapshot",
                        lambda: [(10, "MaxGameStudioLeague.exe"), (11, "cs2-insight-agent-desktop.exe")])
    with pytest.raises(StorageMigrationError, match="still running"):
        migrate_storage(source=source, destination=destination, host_pid=10)
    with pytest.raises(StorageMigrationError, match="still running"):
        migrate_storage(source=source, destination=tmp_path / "out", host_pid=11)


def test_process_gate_allows_current_host_pid_and_completes(tmp_path: Path, monkeypatch):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    monkeypatch.setattr("app.desktop_storage_migration.ensure_backend_stopped", lambda **_kwargs: None)
    monkeypatch.setattr("app.desktop_storage_migration._process_snapshot",
                        lambda: [(1234, "MaxGameStudioLeague.exe")])
    result = migrate_storage(source=source, destination=destination, host_pid=1234)
    assert result.mode == "migrated"


def test_staging_acl_failure_is_fail_closed(tmp_path: Path, monkeypatch):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    monkeypatch.setattr("app.desktop_storage_migration.os.name", "nt")
    monkeypatch.setattr("app.desktop_storage_migration.subprocess.run",
                        lambda *args, **kwargs: (_ for _ in ()).throw(OSError("acl denied")))
    with pytest.raises(StorageMigrationError, match="secure"):
        migrate_storage(source=source, destination=destination, process_check=False)
    assert not destination.exists()


def test_final_source_change_aborts_before_promotion(tmp_path: Path, monkeypatch):
    source, destination = tmp_path / "source", tmp_path / "destination"
    _tree(source)
    calls = 0
    def gate(**_kwargs):
        nonlocal calls
        calls += 1
        if calls == 2:
            (source / "data" / "db.sqlite-wal").write_bytes(b"late change")
    monkeypatch.setattr("app.desktop_storage_migration.ensure_desktop_stopped", gate)
    with pytest.raises(StorageMigrationError, match="changed"):
        migrate_storage(source=source, destination=destination)
    assert not destination.exists()


def test_broken_or_reparse_ancestor_fails_before_resolution(tmp_path: Path, monkeypatch):
    source, destination = tmp_path / "source", tmp_path / "out"
    _tree(source)
    ancestor = source.parent
    original = __import__("app.desktop_storage_migration", fromlist=["_is_reparse"])._is_reparse
    monkeypatch.setattr("app.desktop_storage_migration._is_reparse",
                        lambda path: path == ancestor or original(path))
    with pytest.raises(StorageMigrationError, match="symlinks|reparse"):
        migrate_storage(source=source, destination=destination, process_check=False)


@pytest.mark.parametrize("name", ["cache", "trash"])
def test_legacy_data_junction_is_copied_as_ordinary_directory(tmp_path: Path, name: str):
    root, source, target = _junction_fixture(tmp_path, name)
    try:
        appdata, local, destination = root / "appdata", root / "local", root / "destination"
        legacy_data = appdata / "CS2 Insight Agent" / "data"
        legacy_data.mkdir(parents=True)
        link = legacy_data / name
        # Move the fixture's junction into the legacy source tree.
        _remove_junction(source / "data" / name)
        _make_junction(link, target)
        migrate_storage(legacy_appdata=appdata, legacy_localappdata=local,
                        destination=destination, process_check=False)
        assert (destination / "data" / name / "payload.bin").read_bytes() == b"junction-payload"
        assert (link / "payload.bin").read_bytes() == b"junction-payload"
        assert target.joinpath("payload.bin").is_file()
        assert not (destination / "data" / name).is_symlink()
    finally:
        _remove_junction(appdata / "CS2 Insight Agent" / "data" / name)
        shutil.rmtree(root, ignore_errors=True)


def test_arbitrary_source_root_junction_is_rejected(tmp_path: Path):
    root, source, target = _junction_fixture(tmp_path, "arbitrary")
    try:
        (source / "data" / "real.bin").write_bytes(b"data")
        arbitrary = source / "unknown-link"
        _remove_junction(source / "data" / "arbitrary")
        _make_junction(arbitrary, target)
        with pytest.raises(StorageMigrationError, match="symlinks|junctions"):
            migrate_storage(source=source, destination=root / "destination", process_check=False)
    finally:
        _remove_junction(source / "unknown-link")
        shutil.rmtree(root, ignore_errors=True)


@pytest.mark.parametrize("name", ["cache", "trash"])
def test_nested_junction_under_allowed_alias_is_rejected_without_promotion(tmp_path: Path, name: str):
    root, source, target = _junction_fixture(tmp_path, f"nested-{name}")
    nested_target = root / "nested-target"
    nested_target.mkdir()
    (nested_target / "nested.bin").write_bytes(b"nested")
    nested_link = target / "nested-link"
    _make_junction(nested_link, nested_target)
    try:
        appdata, local, destination = root / "appdata", root / "local", root / "destination"
        data = appdata / "CS2 Insight Agent" / "data"
        data.mkdir(parents=True)
        _remove_junction(source / "data" / f"nested-{name}")
        _make_junction(data / name, target)
        with pytest.raises(StorageMigrationError, match="symlinks|junctions|nested"):
            migrate_storage(legacy_appdata=appdata, legacy_localappdata=local,
                            destination=destination, process_check=False)
        assert not destination.exists()
        assert (data / name).is_dir() and nested_link.exists()
    finally:
        _remove_junction(data / name)
        _remove_junction(nested_link)
        shutil.rmtree(root, ignore_errors=True)


def test_legacy_alias_retarget_during_copy_is_rejected(tmp_path: Path):
    root, source, target = _junction_fixture(tmp_path, "retarget")
    alternate = root / "alternate"
    alternate.mkdir()
    (alternate / "payload.bin").write_bytes(b"alternate")
    try:
        appdata, local, destination = root / "appdata", root / "local", root / "destination"
        data = appdata / "CS2 Insight Agent" / "data"
        data.mkdir(parents=True)
        link = data / "cache"
        _remove_junction(source / "data" / "retarget")
        _make_junction(link, target)
        def copy_and_retarget(src: Path, dst: Path) -> None:
            shutil.copytree(src, dst, dirs_exist_ok=True)
            _remove_junction(link)
            _make_junction(link, alternate)
        with pytest.raises(StorageMigrationError, match="changed"):
            migrate_storage(legacy_appdata=appdata, legacy_localappdata=local,
                            destination=destination, process_check=False, copy_impl=copy_and_retarget)
        assert not destination.exists()
        assert alternate.joinpath("payload.bin").read_bytes() == b"alternate"
    finally:
        _remove_junction(link)
        shutil.rmtree(root, ignore_errors=True)


def test_legacy_alias_to_system_volume_is_rejected(tmp_path: Path, monkeypatch):
    root, source, target = _junction_fixture(tmp_path, "system-target")
    # Reject before filesystem access; never create a fixture on the real
    # system volume, even when pytest's temporary directory is on D:.
    system_target = Path(os.environ.get("SystemDrive", "C:") + "\\mgs-test-rejected-target")
    try:
        appdata, local, destination = root / "appdata", root / "local", root / "destination"
        data = appdata / "CS2 Insight Agent" / "data"
        data.mkdir(parents=True)
        link = data / "cache"
        _remove_junction(source / "data" / "system-target")
        _make_junction(link, target)
        real_readlink = os.readlink
        monkeypatch.setattr("app.desktop_storage_migration.os.readlink",
                            lambda path: str(system_target) if Path(path) == link else real_readlink(path))
        with pytest.raises(StorageMigrationError, match="non-system volume"):
            migrate_storage(legacy_appdata=appdata, legacy_localappdata=local,
                            destination=destination, process_check=False)
        assert not destination.exists()
        assert link.is_dir() and target.joinpath("payload.bin").read_bytes() == b"junction-payload"
    finally:
        _remove_junction(link)
        shutil.rmtree(root, ignore_errors=True)


def test_dangling_reparse_ancestor_is_rejected_by_lstat(tmp_path: Path, monkeypatch):
    dangling = tmp_path / "dangling-junction"
    candidate = dangling / "child"
    real_lstat, real_stat = os.lstat, os.stat
    fake_meta = types.SimpleNamespace(st_file_attributes=0x400, st_mode=stat.S_IFDIR)
    monkeypatch.setattr("app.desktop_storage_migration.os.lstat",
                        lambda path: fake_meta if Path(path) == dangling else real_lstat(path))
    monkeypatch.setattr("app.desktop_storage_migration.os.stat",
                        lambda path, **kwargs: fake_meta if Path(path) == dangling else real_stat(path, **kwargs))
    with pytest.raises(StorageMigrationError, match="symlinks"):
        __import__("app.desktop_storage_migration", fromlist=["_safe_resolve"])._safe_resolve(candidate)


def test_fresh_layout_marker_is_idempotent(tmp_path: Path):
    destination = tmp_path / "new"
    kwargs = {"legacy_appdata": tmp_path / "appdata", "legacy_localappdata": tmp_path / "local",
              "destination": destination, "process_check": False}
    assert migrate_storage(**kwargs).mode == "migrated"
    assert migrate_storage(**kwargs).mode == "existing"


def test_fresh_layout_invokes_staging_security_gate(tmp_path: Path, monkeypatch):
    destination = tmp_path / "new"
    calls = []
    monkeypatch.setattr("app.desktop_storage_migration._secure_staging", lambda path: calls.append(path))
    migrate_storage(legacy_appdata=tmp_path / "appdata", legacy_localappdata=tmp_path / "local",
                    destination=destination, process_check=False)
    # Empty installs still pass through the same staged ACL boundary.
    assert len(calls) == 1
    assert calls[0].parent == destination.parent
