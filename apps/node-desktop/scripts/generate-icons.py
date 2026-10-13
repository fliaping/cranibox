"""Generate the code-defined node icon without external graphics dependencies."""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1] / 'src-tauri/icons'
ROOT.mkdir(parents=True, exist_ok=True)

def png(size):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            u, v = x / size, y / size
            color = (15, 23, 42, 255)
            if (0.47 <= u <= 0.53 and 0.28 <= v <= 0.66) or (0.26 <= u <= 0.74 and 0.61 <= v <= 0.67):
                color = (248, 250, 252, 255)
            for cx, cy in ((0.5, 0.26), (0.26, 0.7), (0.74, 0.7)):
                if abs(u - cx) <= 0.115 and abs(v - cy) <= 0.115:
                    color = (34, 197, 94, 255)
            rows.extend(color)
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(rows))) + chunk(b'IEND', b'')

for size, name in ((32, '32x32.png'), (128, '128x128.png'), (256, '128x128@2x.png')):
    (ROOT / name).write_bytes(png(size))
data = png(256)
(ROOT / 'icon.ico').write_bytes(struct.pack('<HHH', 0, 1, 1) + struct.pack('<BBBBHHII', 0, 0, 0, 0, 1, 32, len(data), 22) + data)
parts = b''.join(kind + struct.pack('>I', len(data) + 8) + data for kind, data in ((b'ic08', png(256)), (b'ic09', png(512))))
(ROOT / 'icon.icns').write_bytes(b'icns' + struct.pack('>I', len(parts) + 8) + parts)
