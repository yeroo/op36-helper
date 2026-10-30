"""Capture the OP36 firmware log from its USB serial port.

Usage: .venv/Scripts/python -I firmware/tools/serial_log.py COM6 [seconds] [outfile]
"""
import sys
import time

import serial

port = sys.argv[1]
secs = int(sys.argv[2]) if len(sys.argv) > 2 else 60
out = open(sys.argv[3], "ab") if len(sys.argv) > 3 else None
with serial.Serial(port, 115200, timeout=0.2) as s:
    end = time.monotonic() + secs
    while time.monotonic() < end:
        data = s.read(4096)
        if data:
            sys.stdout.buffer.write(data)
            sys.stdout.flush()
            if out:
                out.write(data)
                out.flush()
