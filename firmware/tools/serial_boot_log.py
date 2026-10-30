"""Wait for the OP36 serial port to (re)appear, then record its log. Usage: serial_boot_log.py COM6 seconds outfile"""
import sys, time, serial
port, secs, outfile = sys.argv[1], int(sys.argv[2]), sys.argv[3]
deadline = time.monotonic() + secs
out = open(outfile, "ab")
while time.monotonic() < deadline:
    try:
        with serial.Serial(port, 115200, timeout=0.2) as s:
            out.write(b"\n--- port opened ---\n"); out.flush()
            while time.monotonic() < deadline:
                data = s.read(4096)
                if data:
                    out.write(data); out.flush()
    except Exception as e:  # port gone during reset: retry
        out.write(f"\n--- {type(e).__name__} ---\n".encode()); out.flush()
        time.sleep(0.5)
