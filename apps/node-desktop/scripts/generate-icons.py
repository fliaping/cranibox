"""Generate the unified Cranibox Boxes brand icons without external graphics dependencies."""
from pathlib import Path
import math
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1] / "src-tauri/icons"
ROOT.mkdir(parents=True, exist_ok=True)
FAVICON = Path(__file__).resolve().parents[3] / "addons/control-center/favicon.png"


def arc_points(x1, y1, rx, ry, x2, y2, steps=16):
    """Sample an SVG circular arc (large_arc=0, sweep=0, r=rx=ry) into polyline points."""
    dx, dy = (x1 - x2) / 2.0, (y1 - y2) / 2.0
    d2 = dx * dx + dy * dy
    r = rx
    h = math.sqrt(max(0.0, r * r - d2 / 1.0))
    # For sweep=0 (counter-clockwise in y-down screen coordinates), center is offset to the right of (x1->x2)
    dist = math.sqrt(d2) * 2.0
    mx, my = (x1 + x2) / 2.0, (y1 + y2) / 2.0
    ux, uy = (x2 - x1) / dist, (y2 - y1) / dist
    # Normal for sweep=0, large_arc=0: (uy, -ux)
    cx = mx + h * uy
    cy = my - h * ux
    a1 = math.atan2(y1 - cy, x1 - cx)
    a2 = math.atan2(y2 - cy, x2 - cx)
    da = a2 - a1
    while da > 0:
        da -= 2.0 * math.pi
    if abs(da) > math.pi:
        # Fallback if angle wrapped
        cx = mx - h * uy
        cy = my + h * ux
        a1 = math.atan2(y1 - cy, x1 - cx)
        a2 = math.atan2(y2 - cy, x2 - cx)
        da = a2 - a1
        while da > math.pi:
            da -= 2.0 * math.pi
        while da < -math.pi:
            da += 2.0 * math.pi
    return [
        (cx + r * math.cos(a1 + da * i / steps), cy + r * math.sin(a1 + da * i / steps))
        for i in range(1, steps + 1)
    ]


def build_boxes_segments():
    """Return exact line segments for the 12 paths of Lucide Boxes in 24x24 coordinates."""
    polylines = []
    # Path 1: M2.97 12.92A2 2 0 0 0 2 14.63v3.24a2 2 0 0 0 .97 1.71l3 1.8a2 2 0 0 0 2.06 0L12 19v-5.5l-5-3-4.03 2.42Z
    p1 = [(2.97, 12.92)]
    p1 += arc_points(2.97, 12.92, 2.0, 2.0, 2.0, 14.63)
    p1.append((2.0, 17.87))
    p1 += arc_points(2.0, 17.87, 2.0, 2.0, 2.97, 19.58)
    p1.append((5.97, 21.38))
    p1 += arc_points(5.97, 21.38, 2.0, 2.0, 8.03, 21.38)
    p1 += [(12.0, 19.0), (12.0, 13.5), (7.0, 10.5), (2.97, 12.92)]
    polylines.append(p1)

    # Inner lines of bottom-left cube
    polylines.append([(7.0, 16.5), (2.26, 13.65)])
    polylines.append([(7.0, 16.5), (12.0, 13.5)])
    polylines.append([(7.0, 16.5), (7.0, 21.67)])

    # Path 5: M12 13.5V19l3.97 2.38a2 2 0 0 0 2.06 0l3-1.8a2 2 0 0 0 .97-1.71v-3.24a2 2 0 0 0-.97-1.71L17 10.5l-5 3Z
    p5 = [(12.0, 13.5), (12.0, 19.0), (15.97, 21.38)]
    p5 += arc_points(15.97, 21.38, 2.0, 2.0, 18.03, 21.38)
    p5.append((21.03, 19.58))
    p5 += arc_points(21.03, 19.58, 2.0, 2.0, 22.0, 17.87)
    p5.append((22.0, 14.63))
    p5 += arc_points(22.0, 14.63, 2.0, 2.0, 21.03, 12.92)
    p5 += [(17.0, 10.5), (12.0, 13.5)]
    polylines.append(p5)

    # Inner lines of bottom-right cube
    polylines.append([(17.0, 16.5), (12.0, 13.5)])
    polylines.append([(17.0, 16.5), (21.74, 13.65)])
    polylines.append([(17.0, 16.5), (17.0, 21.67)])

    # Path 9: M7.97 4.42A2 2 0 0 0 7 6.13v4.37l5 3 5-3V6.13a2 2 0 0 0-.97-1.71l-3-1.8a2 2 0 0 0-2.06 0l-3 1.8Z
    p9 = [(7.97, 4.42)]
    p9 += arc_points(7.97, 4.42, 2.0, 2.0, 7.0, 6.13)
    p9 += [(7.0, 10.5), (12.0, 13.5), (17.0, 10.5), (17.0, 6.13)]
    p9 += arc_points(17.0, 6.13, 2.0, 2.0, 16.03, 4.42)
    p9.append((13.03, 2.62))
    p9 += arc_points(13.03, 2.62, 2.0, 2.0, 10.97, 2.62)
    p9.append((7.97, 4.42))
    polylines.append(p9)

    # Inner lines of top cube
    polylines.append([(12.0, 8.0), (7.26, 5.15)])
    polylines.append([(12.0, 8.0), (16.74, 5.15)])
    polylines.append([(12.0, 13.5), (12.0, 8.0)])

    segments = []
    for poly in polylines:
        for i in range(len(poly) - 1):
            ax, ay = poly[i]
            bx, by = poly[i + 1]
            dx, dy = bx - ax, by - ay
            len2 = dx * dx + dy * dy
            if len2 > 1e-9:
                segments.append((ax, ay, dx, dy, len2))
    return segments


SEGMENTS = build_boxes_segments()


def min_dist_to_boxes(vx, vy):
    """Return Euclidean distance in 24x24 space from (vx, vy) to the Boxes centerline."""
    best2 = 1e9
    for ax, ay, dx, dy, len2 in SEGMENTS:
        t = ((vx - ax) * dx + (vy - ay) * dy) / len2
        if t < 0.0:
            t = 0.0
        elif t > 1.0:
            t = 1.0
        px = ax + t * dx - vx
        py = ay + t * dy - vy
        d2 = px * px + py * py
        if d2 < best2:
            best2 = d2
    return math.sqrt(best2)


def pack_png(size, rows):
    def chunk(kind, data):
        return (
            struct.pack(">I", len(data))
            + kind
            + data
            + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
        + chunk(b"IEND", b"")
    )


def clamp01(x):
    return 0.0 if x <= 0.0 else (1.0 if x >= 1.0 else x)


def render_brand_png(size, pad_ratio=0.0):
    """Render the .cx-brand-mark green squircle + white Boxes icon at (size x size)."""
    rows = bytearray()
    pad = size * pad_ratio
    sq_size = size - 2.0 * pad
    half = sq_size * 0.5
    radius = sq_size * (11.0 / 38.0)  # Exact .cx-brand-mark curvature (11px on 38px)
    inner = half - radius
    # Map 24x24 Boxes viewBox into the squircle (matching 21px inside 38px .cx-brand-mark)
    glyph_span = sq_size * 0.62
    scale = glyph_span / 24.0
    inv_scale = 1.0 / scale
    aa = 0.75  # Anti-aliasing width in output pixels
    stroke_half = 1.0  # stroke-width="2" -> radius 1.0 in 24x24 space

    cx = size * 0.5
    cy = size * 0.5

    for y in range(size):
        rows.append(0)
        py = y + 0.5
        t_grad = clamp01((py - pad) / max(1.0, sq_size))
        # Subtle vertical emerald gradient #059669 (5, 150, 105) -> #047857 (4, 120, 87)
        bg_r = 5.0 * (1.0 - t_grad) + 4.0 * t_grad
        bg_g = 150.0 * (1.0 - t_grad) + 120.0 * t_grad
        bg_b = 105.0 * (1.0 - t_grad) + 87.0 * t_grad

        dy = abs(py - cy) - inner
        vy = (py - cy) * inv_scale + 12.0

        for x in range(size):
            px = x + 0.5
            dx = abs(px - cx) - inner
            ox = max(dx, 0.0)
            oy = max(dy, 0.0)
            sd_sq = math.hypot(ox, oy) + min(max(dx, dy), 0.0) - radius
            alpha_bg = clamp01(0.5 - sd_sq / 1.0)
            if alpha_bg <= 0.0:
                rows.extend((0, 0, 0, 0))
                continue

            vx = (px - cx) * inv_scale + 12.0
            if 0.5 <= vx <= 23.5 and 0.5 <= vy <= 23.5:
                dist_px = (min_dist_to_boxes(vx, vy) - stroke_half) * scale
                alpha_fg = clamp01(0.5 - dist_px / aa)
            else:
                alpha_fg = 0.0

            r = int(round(bg_r * (1.0 - alpha_fg) + 255.0 * alpha_fg))
            g = int(round(bg_g * (1.0 - alpha_fg) + 255.0 * alpha_fg))
            b = int(round(bg_b * (1.0 - alpha_fg) + 255.0 * alpha_fg))
            a = int(round(alpha_bg * 255.0))
            rows.extend((r, g, b, a))

    return pack_png(size, rows)


def render_tray_template_png(size=44):
    """Render monochrome black + alpha macOS menu bar template icon for Boxes."""
    rows = bytearray()
    scale = (size * 0.82) / 24.0
    inv_scale = 1.0 / scale
    cx = size * 0.5
    cy = size * 0.5
    stroke_half = 1.0

    for y in range(size):
        rows.append(0)
        vy = (y + 0.5 - cy) * inv_scale + 12.0
        for x in range(size):
            vx = (x + 0.5 - cx) * inv_scale + 12.0
            dist_px = (min_dist_to_boxes(vx, vy) - stroke_half) * scale
            alpha = clamp01(0.5 - dist_px / 0.8)
            rows.extend((0, 0, 0, int(round(alpha * 255.0))))
    return pack_png(size, rows)


png_16 = render_brand_png(16, pad_ratio=0.0)
png_24 = render_brand_png(24, pad_ratio=0.0)
png_32 = render_brand_png(32, pad_ratio=0.0)
png_48 = render_brand_png(48, pad_ratio=0.04)
png_64 = render_brand_png(64, pad_ratio=0.04)
png_128 = render_brand_png(128, pad_ratio=0.06)
png_256 = render_brand_png(256, pad_ratio=0.07)
png_512 = render_brand_png(512, pad_ratio=0.08)

(ROOT / "32x32.png").write_bytes(png_32)
(ROOT / "128x128.png").write_bytes(png_128)
(ROOT / "128x128@2x.png").write_bytes(png_256)
(ROOT / "tray-template.png").write_bytes(render_tray_template_png(44))
if FAVICON.parent.exists():
    FAVICON.write_bytes(png_64)

# Multi-resolution Windows ICO (16, 24, 32, 48, 64, 256)
ico_images = [(16, png_16), (24, png_24), (32, png_32), (48, png_48), (64, png_64), (256, png_256)]
ico_header = bytearray(struct.pack("<HHH", 0, 1, len(ico_images)))
offset = 6 + 16 * len(ico_images)
ico_payload = bytearray()
for sz, blob in ico_images:
    w_byte = 0 if sz == 256 else sz
    ico_header.extend(struct.pack("<BBBBHHII", w_byte, w_byte, 0, 0, 1, 32, len(blob), offset))
    ico_payload.extend(blob)
    offset += len(blob)
(ROOT / "icon.ico").write_bytes(bytes(ico_header + ico_payload))

# macOS ICNS (ic11=32, ic12=64, ic07=128, ic08=256, ic13=256, ic09=512, ic14=512)
icns_parts = b"".join(
    kind + struct.pack(">I", len(blob) + 8) + blob
    for kind, blob in (
        (b"ic11", png_32),
        (b"ic12", png_64),
        (b"ic07", png_128),
        (b"ic08", png_256),
        (b"ic13", png_256),
        (b"ic09", png_512),
        (b"ic14", png_512),
    )
)
(ROOT / "icon.icns").write_bytes(b"icns" + struct.pack(">I", len(icns_parts) + 8) + icns_parts)
