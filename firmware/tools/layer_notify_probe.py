"""Probe the OP36 layer-notify protocol over the Vial raw-HID interface.

Usage (from the omega36 repo root, keyboard on USB):
    .venv/Scripts/python -I firmware/tools/layer_notify_probe.py            # subscribe, print state + notifications for 15 s
    .venv/Scripts/python -I firmware/tools/layer_notify_probe.py --listen   # no subscribe, just print E9 packets for 10 s
    add --secs=N to change the listening time; use python -u when piping output
Unit test:
    .venv/Scripts/python -I -m pytest firmware/tools/layer_notify_probe.py -q
"""

import sys
import time

VID, PID = 0xE126, 0x0036
USAGE_PAGE, USAGE = 0xFF60, 0x61
NAMESPACE, SUB_STATE = 0xE9, 0x01


def parse_state(pkt: bytes) -> dict:
    assert pkt[0] == NAMESPACE and pkt[1] == SUB_STATE, f"not a layer-state packet: {pkt[:4].hex()}"
    return {"active": pkt[2], "default": pkt[3], "mask": pkt[4] | (pkt[5] << 8), "fw_layout": pkt[6]}


def fmt(tag: str, pkt: bytes) -> str:
    s = parse_state(pkt)
    return f"{tag}: active={s['active']} default={s['default']} mask={s['mask']:#b} fw_layout={s['fw_layout']}"


def open_device():
    import hid

    for info in hid.enumerate(VID, PID):
        if info["usage_page"] == USAGE_PAGE and info["usage"] == USAGE:
            dev = hid.device()
            dev.open_path(info["path"])
            return dev
    sys.exit("OP36 Vial interface not found (is the keyboard on USB?)")


def main(argv: list[str]) -> None:
    listen_only = "--listen" in argv
    dev = open_device()
    dev.set_nonblocking(False)
    def subscribe(ttl: int) -> bytes:
        dev.write(bytes([0x00, 0x07, NAMESPACE, SUB_STATE, ttl]) + bytes(28))
        return bytes(dev.read(32, 1000))

    if not listen_only:
        reply = subscribe(60)
        print(fmt("reply", reply) if reply[:2] == bytes([NAMESPACE, SUB_STATE]) else f"unexpected reply: {reply.hex()}")
    secs = next((int(a.split("=", 1)[1]) for a in argv if a.startswith("--secs=")), 10 if listen_only else 15)
    start = time.monotonic()
    deadline = start + secs
    last_renew = start
    print("listening... hold / release the thumb layer keys", flush=True)
    while time.monotonic() < deadline:
        if not listen_only and time.monotonic() - last_renew >= 20:  # same cadence as the helper: TTL 60, renew every 20 s
            subscribe(60)
            last_renew = time.monotonic()
            print(f"renewed at +{last_renew - start:.0f}s", flush=True)
        pkt = bytes(dev.read(32, 200))
        if len(pkt) >= 7 and pkt[0] == NAMESPACE:
            print(fmt("notify", pkt), flush=True)
    dev.close()


if __name__ == "__main__":
    main(sys.argv[1:])


def test_parse_state():
    assert parse_state(bytes([0xE9, 1, 3, 0, 0x0E, 0, 1] + [0] * 25)) == {
        "active": 3,
        "default": 0,
        "mask": 0b1110,
        "fw_layout": 1,
    }
