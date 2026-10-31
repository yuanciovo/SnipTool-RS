#!/usr/bin/env python3
"""生成 MySnipTool 的图标资源（应用图标 + 托盘单色版）。

几何取自 assets/icons/variants/v1-round.png（512px 原图实测）：
  - 字形盒 408px / 画布 512px（上下左右各留 52px）
  - 笔画 32px = 字形盒的 7.8%
  - 两笔：左上角交叉、右下角交叉，右上和左下是 90° 圆角

原图 7.8% 的笔画在 16px 下只有 1.25px —— 缩放后会糊成一团。
所以这里做**光学补偿**：尺寸越小笔画占比越大（16px 用 15% ≈ 2.4px，32px 用 13.5%，
64px 起逐步回到原比例 9.3%）。加粗过头会把中间的空洞填死，15% 是实测的甜点。

输出：
  assets/app.ico          16/20/24/32/40/48/64/256（96/128 故意不做：+105KB 不值）
  assets/tray-dark.ico    深色字形（浅色任务栏用）16/20/24/32
  assets/tray-light.ico   白色字形（深色任务栏用）16/20/24/32
  assets/icons/preview.png  一排尺寸预览，肉眼验货用
"""

import math
import os
import struct
from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ASSETS = os.path.join(ROOT, "assets")

BLUE = (11, 99, 246, 255)      # #0B63F6，取自原图笔画中心
DARK = (31, 31, 31, 255)       # #1F1F1F
WHITE = (255, 255, 255, 255)

# 字形盒归一化坐标
C_L, C_R = 0.195, 0.805
C_T, C_B = 0.195, 0.805
CORNER = 0.090


def stroke_ratio(size: int) -> float:
    """笔画占字形盒的比例。小尺寸加粗，大字回归原始比例。"""
    if size <= 20:
        return 0.150
    if size <= 32:
        return 0.135
    if size <= 64:
        return 0.110
    return 0.093


def render(size: int, color, ss: int = 8) -> Image.Image:
    """渲染一张 size×size 的 RGBA 图标，ss 倍超采样后缩小。"""
    big = size * ss
    margin = max(1.0, size * 0.06) * ss
    box = big - 2 * margin

    def p(nx, ny):
        return (margin + nx * box, margin + ny * box)

    r = CORNER
    w = max(2.0, stroke_ratio(size) * box)
    half = w / 2.0

    # 路径 A：上横 → 右上圆角 → 右竖
    a = [p(0.0, C_T)]
    steps = 16
    for i in range(steps + 1):
        t = i / steps
        ang = (-90 + 90 * t) * 3.141592653589793 / 180.0
        cx, cy = p(C_R - r, C_T + r)
        a.append((cx + r * box * math.cos(ang), cy + r * box * math.sin(ang)))
    a.append(p(C_R, 1.0))

    # 路径 B：左竖 → 左下圆角 → 下横
    b = [p(C_L, 0.0)]
    for i in range(steps + 1):
        t = i / steps
        ang = (180 - 90 * t) * 3.141592653589793 / 180.0
        cx, cy = p(C_L + r, C_B - r)
        b.append((cx + r * box * math.cos(ang), cy + r * box * math.sin(ang)))
    b.append(p(1.0, C_B))

    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    for path in (a, b):
        d.line(path, fill=color, width=int(round(w)), joint="curve")
        for pt in (path[0], path[-1]):
            d.ellipse(
                [pt[0] - half, pt[1] - half, pt[0] + half, pt[1] + half], fill=color
            )

    return img.resize((size, size), Image.LANCZOS)


def bmp_entry(img: Image.Image) -> bytes:
    """32bpp 未压缩的 ICO 图像数据（BITMAPINFOHEADER + BGRA + AND 掩码）。"""
    w, h = img.size
    header = struct.pack(
        "<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, w * h * 4, 0, 0, 0, 0
    )
    px = img.load()
    rows = []
    for y in range(h - 1, -1, -1):
        row = bytearray()
        for x in range(w):
            r, g, b, a = px[x, y]
            row += bytes((b, g, r, a))
        rows.append(bytes(row))
    xor = b"".join(rows)
    mask_stride = ((w + 31) // 32) * 4
    return header + xor + bytes(mask_stride * h)


def write_ico(path: str, images):
    """自己打包 ICO：小尺寸用 BMP（兼容性最好），256 用 PNG（体积小）。"""
    import io

    entries, blobs = [], []
    offset = 6 + 16 * len(images)
    for im in images:
        if im.width >= 256:
            buf = io.BytesIO()
            im.save(buf, format="PNG", optimize=True)
            data = buf.getvalue()
        else:
            data = bmp_entry(im)
        entries.append((im.width, im.height, len(data), offset))
        blobs.append(data)
        offset += len(data)

    out = bytearray()
    out += struct.pack("<HHH", 0, 1, len(images))
    for (w, h, size, off) in entries:
        out += struct.pack(
            "<BBBBHHII",
            w if w < 256 else 0,
            h if h < 256 else 0,
            0,
            0,
            1,
            32,
            size,
            off,
        )
    for blob in blobs:
        out += blob
    with open(path, "wb") as f:
        f.write(out)


def main():
    # 96/128 故意不做：资源管理器会用 256 缩放，代价是 +105KB 体积。
    # 对这个"体积就是卖点"的项目，不值得。
    app_sizes = [16, 20, 24, 32, 40, 48, 64, 256]
    app_imgs = [render(s, BLUE) for s in app_sizes]
    write_ico(os.path.join(ASSETS, "app.ico"), app_imgs)

    tray_sizes = [16, 20, 24, 32]
    write_ico(
        os.path.join(ASSETS, "tray-dark.ico"),
        [render(s, DARK, ss=16) for s in tray_sizes],
    )
    write_ico(
        os.path.join(ASSETS, "tray-light.ico"),
        [render(s, WHITE, ss=16) for s in tray_sizes],
    )

    # 预览条：每个尺寸塞进 136px 的格子，最近邻放大，浅色底
    cell = 136
    preview = Image.new("RGBA", (cell * len(app_sizes), cell), (245, 245, 245, 255))
    for i, (s, im) in enumerate(zip(app_sizes, app_imgs)):
        k = max(1, min(cell // s - 1, 8))
        big = im.resize((s * k, s * k), Image.NEAREST)
        preview.alpha_composite(big, (i * cell + (cell - big.width) // 2, (cell - big.height) // 2))
    os.makedirs(os.path.join(ASSETS, "icons"), exist_ok=True)
    preview.save(os.path.join(ASSETS, "icons", "preview.png"))

    for s, im in zip(app_sizes, app_imgs):
        im.save(os.path.join(ASSETS, "icons", "app-%d.png" % s))

    total = sum(os.path.getsize(os.path.join(ASSETS, n)) for n in
                ("app.ico", "tray-dark.ico", "tray-light.ico"))
    print("app.ico / tray-dark.ico / tray-light.ico -> %d bytes total" % total)
    print("preview: assets/icons/preview.png")


if __name__ == "__main__":
    main()
