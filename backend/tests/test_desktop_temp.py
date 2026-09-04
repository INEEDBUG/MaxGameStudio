import tempfile

import pytest

from app.desktop_temp import configure_desktop_temp


def test_selected_directory_is_pinned_even_if_environment_changes(tmp_path, monkeypatch):
    monkeypatch.setattr(tempfile, "tempdir", None)
    monkeypatch.setenv("CS2_INSIGHT_TEMP_DIR", str(tmp_path))
    configure_desktop_temp()
    monkeypatch.setenv("TEMP", str(tmp_path / "unavailable"))
    assert tempfile.gettempdir() == str(tmp_path)
    with tempfile.NamedTemporaryFile() as probe:
        assert tmp_path == __import__("pathlib").Path(probe.name).parent


def test_missing_selected_directory_refuses_fallback(tmp_path, monkeypatch):
    monkeypatch.setenv("CS2_INSIGHT_TEMP_DIR", str(tmp_path / "missing"))
    with pytest.raises(RuntimeError, match="refusing fallback"):
        configure_desktop_temp()


def test_removed_directory_does_not_fall_back(tmp_path, monkeypatch):
    selected = tmp_path / "selected"
    selected.mkdir()
    monkeypatch.setattr(tempfile, "tempdir", None)
    monkeypatch.setenv("CS2_INSIGHT_TEMP_DIR", str(selected))
    configure_desktop_temp()
    selected.rmdir()
    with pytest.raises(FileNotFoundError):
        tempfile.NamedTemporaryFile()


def test_server_without_desktop_selection_is_unchanged(monkeypatch):
    monkeypatch.delenv("CS2_INSIGHT_TEMP_DIR", raising=False)
    monkeypatch.setattr(tempfile, "tempdir", "server-owned-path")
    configure_desktop_temp()
    assert tempfile.tempdir == "server-owned-path"


def test_relative_path_rejected(monkeypatch):
    monkeypatch.setenv("CS2_INSIGHT_TEMP_DIR", "relative")
    with pytest.raises(RuntimeError):
        configure_desktop_temp()
