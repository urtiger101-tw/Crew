"""Cleanup of test-owned temporary directories after native ConPTY shutdown."""
import os
from pathlib import Path
import tempfile
import time


class SmokeDirectory(tempfile.TemporaryDirectory):
    def __init__(self, *, prefix):
        if not prefix.startswith("crew-") or any(c in prefix for c in "\\/:"):
            raise ValueError("expected a Crew test directory prefix")
        self._intended_parent = Path(tempfile.gettempdir()).resolve()
        super().__init__(prefix=prefix)
        self._owned_path = Path(self.name).resolve()
        self._check_target()

    def _check_target(self):
        target = Path(self.name).resolve()
        if target != self._owned_path or target.parent != self._intended_parent:
            raise ValueError("temporary cleanup target escaped its owned directory")

    def cleanup(self):
        deadline = time.monotonic() + 5
        while True:
            self._check_target()
            try:
                super().cleanup()
                return
            except PermissionError as error:
                # IPC can close before Windows releases a killed shell's cwd.
                # Never ignore another error or an unreleased handle at deadline.
                if os.name != "nt" or getattr(error, "winerror", None) != 32 or time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)
