"""Private offline PTY fixture: record every input character without sending commands."""
import os
from pathlib import Path
import sys
import threading
import time

root = Path(os.environ["CREW_AF_TEST_ROOT"])
pane = os.environ["CREW_PANE_ID"]
record = root / (pane + ".input")
record.write_bytes(b"")


def output_signal():
    signal = root / (pane + ".output")
    count = 0
    while True:
        if signal.exists():
            signal.unlink()
            count += 1
            print(f"FAKE_OUTPUT {count}", flush=True)
        time.sleep(0.02)


threading.Thread(target=output_signal, daemon=True).start()
print("FAKE_READY", flush=True)
if os.name == "nt":
    import msvcrt
    while True:
        value = msvcrt.getwch().encode("utf-8")
        with record.open("ab") as stream:
            stream.write(value)
else:
    import tty
    tty.setraw(sys.stdin.fileno())
    while value := os.read(sys.stdin.fileno(), 1024):
        with record.open("ab") as stream:
            stream.write(value)
