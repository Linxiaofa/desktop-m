"""Generate the small, dependency-free Windows app icon used by tauri-build."""

import binascii
import pathlib
import struct
import zlib

SIZE = 64
OUT = pathlib.Path(__file__).resolve().parents[1] / "src-tauri" / "icons" / "icon.ico"
GLYPHS = {
    "D": ["11110", "10001", "10001", "10001", "10001", "10001", "11110"],
    "M": ["10001", "11011", "10101", "10101", "10001", "10001", "10001"],
}


def chunk(kind: bytes, payload: bytes) -> bytes:
    data = kind + payload
    return struct.pack(">I", len(payload)) + data + struct.pack(
        ">I", binascii.crc32(data) & 0xFFFFFFFF
    )


def pixel(x: int, y: int) -> tuple[int, int, int, int]:
    if min(x, y, SIZE - 1 - x, SIZE - 1 - y) < 3:
        return (16, 80, 170, 255)
    for letter, offset in [("D", 6), ("M", 34)]:
        gx = (x - offset) // 4
        gy = (y - 18) // 4
        if 0 <= gx < 5 and 0 <= gy < 7 and GLYPHS[letter][gy][gx] == "1":
            return (255, 255, 255, 255)
    return (23, 109, 230, 255)


rows = b"".join(
    b"\0" + bytes(channel for x in range(SIZE) for channel in pixel(x, y))
    for y in range(SIZE)
)
png = (
    b"\x89PNG\r\n\x1a\n"
    + chunk(b"IHDR", struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0))
    + chunk(b"IDAT", zlib.compress(rows, 9))
    + chunk(b"IEND", b"")
)
ico = (
    struct.pack("<HHH", 0, 1, 1)
    + struct.pack("<BBBBHHII", SIZE, SIZE, 0, 0, 1, 32, len(png), 22)
    + png
)
OUT.parent.mkdir(parents=True, exist_ok=True)
OUT.write_bytes(ico)
print(OUT)
