#!/usr/bin/env python3
"""生成 Sniptool 图标候选（4 种风格）+ 尺寸阶梯预览 + 托盘单色版 + .ico。

图形全部矢量绘制（resvg 渲染），复用 skill-icon-generator 的底板/渐变/高光
处理，只把 mark 换成截图工具专用的几何。

用法：
    python tools/render_icon_candidates.py
输出：
    assets/icons/candidate-*.png     512×512 方角 + 圆角
    assets/icons/sizes/              各候选原生小尺寸（评估托盘可读性）
    assets/icons/ico/                多尺寸 .ico
    assets/icons/tray-mono/          托盘单色（透明底，白/黑两套）
    assets/icons/preview.png         对比板
"""
from __future__ import annotations

import importlib.util
import struct
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
SKILL = Path.home() / ".workbuddy" / "skills" / "skill-icon-generator" / "scripts" / "generate_icon.py"
OUT = ROOT / "assets" / "icons"

FONT_CN = "C:/Windows/Fonts/msyh.ttc"
FONT_EN = "C:/Windows/Fonts/segoeui.ttf"


def _load_renderer():
    spec = importlib.util.spec_from_file_location("gen_icon", SKILL)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


gi = _load_renderer()

# ---------------------------------------------------------------------------
# 图形（24×24 坐标空间）。截图工具的语义母题：取景框 / 快门 / 裁切。
# ---------------------------------------------------------------------------
VIEWFINDER_LINE = (
    '<path fill="none" stroke="{{FG}}" stroke-width="2.7" stroke-linecap="round" '
    'stroke-linejoin="round" d="M2.8 9.6V6.2A3.4 3.4 0 0 1 6.2 2.8h3.4'
    'M14.4 2.8h3.4a3.4 3.4 0 0 1 3.4 3.4v3.4'
    'M21.2 14.4v3.4a3.4 3.4 0 0 1-3.4 3.4h-3.4'
    'M9.6 21.2H6.2a3.4 3.4 0 0 1-3.4-3.4v-3.4"/>'
    '<circle cx="12" cy="12" r="2.5" fill="{{FG}}"/>'
)

SHUTTER = (
    '<circle cx="12" cy="12" r="8.2" fill="none" stroke="{{FG}}" stroke-width="2.9"/>'
    '<circle cx="12" cy="12" r="3.2" fill="{{FG}}"/>'
)

CROP = (
    '<path fill="none" stroke="{{FG}}" stroke-width="2.6" stroke-linecap="round" '
    'stroke-linejoin="round" d="M6.2 2.6v13.6a2 2 0 0 0 2 2h13.4"/>'
    '<path fill="none" stroke="{{FG}}" stroke-width="2.6" stroke-linecap="round" '
    'stroke-linejoin="round" d="M17.8 21.4V7.8a2 2 0 0 0-2-2H2.4"/>'
)

VIEWFINDER_BLOCK = (
    # 方切臂端 + 直角拼接：粗笔重下若用圆帽，四角会鼓成花瓣状，16px 直接糊掉。
    '<path fill="none" stroke="{{FG}}" stroke-width="3.6" stroke-linecap="butt" '
    'stroke-linejoin="round" d="M2.4 9.8V5.8A3.4 3.4 0 0 1 5.8 2.4H9.8'
    'M14.2 2.4h4A3.4 3.4 0 0 1 21.6 5.8V9.8'
    'M21.6 14.2v4a3.4 3.4 0 0 1-3.4 3.4H14.2'
    'M9.8 21.6H5.8A3.4 3.4 0 0 1 2.4 18.2V14.2"/>'
    '<circle cx="12" cy="12" r="3" fill="{{FG}}"/>'
)

# 托盘单色：按尺寸做光学分级。
# 16px 下若臂太长、笔画太细，四个角会并成一个"缺角的方框"；臂收短、笔画加粗
# 才能保住"四角取景"的识别度。20px 以下也去掉中心点——那一档点会直接糊掉。
def _bracket(arm: float, sw: float, dot: float) -> str:
    end = 24 - arm
    seg = arm - 5.8
    d = (f"M2.4 {arm}V5.8A3.4 3.4 0 0 1 5.8 2.4H{arm}"
         f"M{end:.1f} 2.4h{seg:.1f}A3.4 3.4 0 0 1 21.6 5.8V{arm}"
         f"M21.6 {end:.1f}v{seg:.1f}a3.4 3.4 0 0 1-3.4 3.4H{end:.1f}"
         f"M{arm} 21.6H5.8A3.4 3.4 0 0 1 2.4 18.2V{end:.1f}")
    s = (f'<path fill="none" stroke="{{{{FG}}}}" stroke-width="{sw}" '
         f'stroke-linecap="butt" stroke-linejoin="round" d="{d}"/>')
    if dot:
        s += f'<circle cx="12" cy="12" r="{dot}" fill="{{{{FG}}}}"/>'
    return s


def tray_art(size: int) -> tuple[str, float]:
    """返回 (几何, 缩放)：小尺寸用更短的臂与更大的缩放，把像素用满。"""
    if size < 22:
        return _bracket(9.6, 2.9, 0), 1.00
    if size < 28:
        return _bracket(9.6, 2.9, 2.5), 0.96
    if size < 40:
        return _bracket(9.6, 3.0, 2.7), 0.94
    return _bracket(9.6, 3.0, 2.8), 0.90

CANDIDATES = [
    {
        "id": "a", "name": "取景框 · 蓝渐变", "tag": "线框正形 / 品牌蓝",
        "mark": VIEWFINDER_LINE, "bg": "#0B63F6", "bg2": "#4C9BFF",
        "fg": "#FFFFFF", "gloss": True, "scale": 0.78,
    },
    {
        "id": "b", "name": "快门 · 石墨", "tag": "圆形极简 / 深色",
        "mark": SHUTTER, "bg": "#24282F", "bg2": "#121417",
        "fg": "#FFFFFF", "gloss": True, "scale": 0.79,
    },
    {
        "id": "c", "name": "裁切 · 浅色线框", "tag": "轻量 / 浅底",
        "mark": CROP, "bg": "#FFFFFF", "bg2": "#E7EFFB",
        "fg": "#0B63F6", "gloss": False, "scale": 0.74,
    },
    {
        "id": "d", "name": "取景框 · 粗块", "tag": "负形实心 / 小尺寸最清晰",
        "mark": VIEWFINDER_BLOCK, "bg": "#3B45E8", "bg2": "#7B5CFF",
        "fg": "#FFFFFF", "gloss": True, "scale": 0.76,
    },
]

LADDER = (80, 56, 40, 28, 20, 16)
ICO_SIZES = (16, 20, 24, 32, 48, 64, 128, 256)

LIGHT_BG = "#F3F3F3"
DARK_BG = "#1F1F1F"


# ---------------------------------------------------------------------------
def render(cand: dict, size: int, shape: str = "square", scale: float | None = None) -> Image.Image:
    gi.DEFAULT_SCALE = float(scale if scale is not None else cand.get("scale", 0.78))
    svg = gi._svg(24, 24, cand["mark"], cand["fg"], cand["bg"], size, shape,
                  int(size * 0.22), bg2=cand.get("bg2"), gloss=cand.get("gloss", True))
    return Image.open(BytesIO(gi._render(svg, size))).convert("RGBA")


def render_mono(inner: str, fg: str, size: int, scale: float = 0.92) -> Image.Image:
    """透明底单色图标：不铺底板，按 scale 居中缩放。"""
    s = scale * size / 24.0
    tx = (size - 24 * s) / 2.0
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" width="%d" height="%d" '
        'viewBox="0 0 %d %d"><g transform="translate(%.2f,%.2f) scale(%.4f)">%s</g></svg>'
        % (size, size, size, size, tx, tx, s, inner.replace("{{FG}}", fg))
    )
    return Image.open(BytesIO(gi._render(svg, size))).convert("RGBA")


def write_ico(path: Path, images: list[tuple[int, Image.Image]]) -> None:
    """多尺寸 ICO：每档都用 resvg 原生渲染的 PNG 压入，避免位图缩放发虚。"""
    n = len(images)
    header = struct.pack("<HHH", 0, 1, n)
    entries, body = b"", b""
    offset = 6 + 16 * n
    for size, im in images:
        buf = BytesIO()
        im.save(buf, "PNG")
        data = buf.getvalue()
        w = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(data),
                               offset + len(body))
        body += data
    path.write_bytes(header + entries + body)


def _rounded_mask(size: tuple[int, int], radius: int) -> Image.Image:
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle(
        [0, 0, size[0] - 1, size[1] - 1], radius=radius, fill=255)
    return m


# ---------------------------------------------------------------------------
def build_preview() -> Path:
    W, PAD = 1240, 56
    tile, gap = 172, 40
    name_w, ladder_w, col_gap, zoom_w = 168, 364, 20, 150
    row_h = 152

    x_light = PAD + name_w
    x_dark = x_light + ladder_w + col_gap
    x_zoom = x_dark + ladder_w + col_gap

    y_sec1, y_tile = 118, 146
    y_sec2, y_rows = 400, 440
    y_sec3 = y_rows + row_h * len(CANDIDATES) + 24
    H = y_sec3 + 148 + 96

    canvas = Image.new("RGB", (W, H), "#FFFFFF")
    d = ImageDraw.Draw(canvas)
    f_title = ImageFont.truetype(FONT_EN, 26)
    f_sub = ImageFont.truetype(FONT_CN, 13)
    f_sec = ImageFont.truetype(FONT_CN, 14)
    f_name = ImageFont.truetype(FONT_CN, 15)
    f_meta = ImageFont.truetype(FONT_EN, 11)
    f_tag = ImageFont.truetype(FONT_CN, 11)

    d.text((PAD, 40), "Sniptool — icon candidates", font=f_title, fill="#111318")
    d.text((PAD, 78), "512×512 · 无文字 · 取景框语义，按托盘 16px 可读性逐档验证",
           font=f_sub, fill="#8A8F98")

    # --- 01 512 -----------------------------------------------------------
    d.text((PAD, y_sec1), "01   应用图标 512×512", font=f_sec, fill="#5C636D")
    for i, c in enumerate(CANDIDATES):
        x = PAD + i * (tile + gap)
        im = render(c, 512).resize((tile, tile), Image.LANCZOS)
        canvas.paste(im, (x, y_tile), im)
        # 1px 描边只是画板的分格线，不参与图标本身（C 的浅色底板需要它才看得出边界）
        d.rectangle([x, y_tile, x + tile - 1, y_tile + tile - 1], outline="#E4E4E7")
        d.text((x, y_tile + tile + 14), "%s   %s" % (c["id"].upper(), c["name"]),
               font=f_name, fill="#1B1E24")
        d.text((x, y_tile + tile + 38), "%s → %s" % (c["bg"], c.get("bg2") or c["bg"]),
               font=f_meta, fill="#9AA0A8")

    # --- 02 尺寸阶梯 --------------------------------------------------------
    d.text((PAD, y_sec2), "02   尺寸阶梯   左：浅色任务栏   右：深色任务栏     右侧：16px 原始像素 4× 放大",
           font=f_sec, fill="#5C636D")

    for i, c in enumerate(CANDIDATES):
        y = y_rows + i * row_h
        d.line([(PAD, y - 1), (W - PAD, y - 1)], fill="#EFEFF1", width=1)
        d.text((PAD, y + 46), "%s   %s" % (c["id"].upper(), c["name"]),
               font=f_name, fill="#1B1E24")
        d.text((PAD, y + 70), c["tag"], font=f_tag, fill="#9AA0A8")

        for x0, bg in ((x_light, LIGHT_BG), (x_dark, DARK_BG)):
            d.rounded_rectangle([x0, y + 12, x0 + ladder_w, y + row_h - 16],
                                radius=10, fill=bg)
            cx, base = x0 + 22, y + 112
            for s in LADDER:
                im = render(c, s)
                canvas.paste(im, (cx, base - s), im)
                cx += s + 16

        for half, bg in ((0, LIGHT_BG), (1, DARK_BG)):
            bx = x_zoom + half * 72
            box = Image.new("RGBA", (64, 64), bg)
            box.alpha_composite(render(c, 16).resize((64, 64), Image.NEAREST))
            canvas.paste(box.convert("RGB"), (bx, y + 44),
                         _rounded_mask((64, 64), 8))

    # --- 03 托盘单色 --------------------------------------------------------
    d.text((PAD, y_sec3), "03   托盘单色 tray-mono    透明底，随系统明暗切换；16 / 20 / 24 / 32 / 48",
           font=f_sec, fill="#5C636D")
    d.text((PAD, y_sec3 + 60), "托盘建议", font=f_name, fill="#1B1E24")
    d.text((PAD, y_sec3 + 84), "单色描边 / 无底板", font=f_tag, fill="#9AA0A8")

    for x0, bg, fg in ((x_light, LIGHT_BG, "#1B1E24"), (x_dark, DARK_BG, "#FFFFFF")):
        d.rounded_rectangle([x0, y_sec3 + 24, x0 + ladder_w, y_sec3 + 148],
                            radius=10, fill=bg)
        cx, base = x0 + 22, y_sec3 + 120
        for s in (48, 32, 24, 20, 16):
            inner, sc = tray_art(s)
            im = render_mono(inner, fg, s, sc)
            canvas.paste(im, (cx, base - s), im)
            cx += s + 18

    d.text((x_zoom, y_sec3 + 22), "16px 原始像素 4×", font=f_tag, fill="#5C636D")
    inner16, sc16 = tray_art(16)
    for half, (bg, fg) in enumerate(((LIGHT_BG, "#1B1E24"), (DARK_BG, "#FFFFFF"))):
        bx = x_zoom + half * 72
        box = Image.new("RGBA", (64, 64), bg)
        box.alpha_composite(render_mono(inner16, fg, 16, sc16).resize((64, 64), Image.NEAREST))
        canvas.paste(box.convert("RGB"), (bx, y_sec3 + 46), _rounded_mask((64, 64), 8))

    d.text((PAD, H - 66),
           "彩色底板用于 EXE / 开始菜单 / 安装包；托盘走单色版，缩放与明暗主题都不会失效。",
           font=f_sub, fill="#8A8F98")
    d.text((PAD, H - 44),
           "单色版按尺寸分级：16 / 20px 收短臂、去掉中心点，否则四角会在低分辨率下并成一个缺角方框。",
           font=f_sub, fill="#8A8F98")

    path = OUT / "preview.png"
    canvas.save(path)
    return path


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    for sub in ("sizes", "ico", "tray-mono"):
        (OUT / sub).mkdir(exist_ok=True)

    for c in CANDIDATES:
        stem = "candidate-%s" % c["id"]
        render(c, 512, "square").convert("RGB").save(OUT / ("%s.png" % stem), "PNG")
        render(c, 512, "round").save(OUT / ("%s-round.png" % stem), "PNG")
        for s in (16, 20, 24, 32, 48):
            render(c, s).save(OUT / "sizes" / ("%s-%d.png" % (stem, s)))
        write_ico(OUT / "ico" / ("%s.ico" % stem),
                  [(s, render(c, s)) for s in ICO_SIZES])

    for tone, fg in (("white", "#FFFFFF"), ("dark", "#1B1E24")):
        for s in (16, 20, 24, 32, 48):
            inner, sc = tray_art(s)
            render_mono(inner, fg, s, sc).save(
                OUT / "tray-mono" / ("tray-%s-%d.png" % (tone, s)))

    def tray_ico(size: int) -> Image.Image:
        inner, sc = tray_art(size)
        return render_mono(inner, "#FFFFFF", size, sc)

    write_ico(OUT / "ico" / "sniptool-tray-white.ico",
              [(s, tray_ico(s)) for s in ICO_SIZES])

    print("preview:", build_preview())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
