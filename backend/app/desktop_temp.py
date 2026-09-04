"""Pin desktop temporary files to the selected volume, without fallback."""
import os
import tempfile
from pathlib import Path


def configure_desktop_temp() -> None:
    selected = os.environ.get("CS2_INSIGHT_TEMP_DIR")
    if selected is None:
        return  # CLI/server deployments keep their existing platform behavior.
    path = Path(selected)
    if not path.is_absolute() or not path.is_dir():
        raise RuntimeError("Selected desktop temporary directory is unavailable; refusing fallback")
    # Supplying dir explicitly never falls back to the OS temporary directory.
    with tempfile.TemporaryFile(dir=path) as probe:
        probe.write(b"storage-check")
        probe.flush()
    tempfile.tempdir = str(path)
