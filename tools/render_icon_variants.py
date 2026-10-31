#!/usr/bin/env python3
"""候选 C（浅底 + 蓝细线）的同风格变体组。

配色、底板、圆角、高光全部复用 candidate-c 的口径，只换图形语言：
1. v1 指定版：光圈放大、线条变细、四臂推到画布边缘
2. v2 大取景框
3. v3 虚线圈选
4. v4 斜切角框
5. v5 偏移双框
6. v6 光圈快门

输出：
    assets/icons/variants/v*-round.png / v*.png   512×512
    assets/icons/variants/preview.png             对比板
"""
from __future__ import annotations

import sys
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, str(Path(__file__).resolve().parent))
import render_icon_candidates as base  # noqa: E402  (复用 skill 渲染器)

ROOT = base.ROOT
OUT = ROOT / "assets" / "icons" / "variants"

FONT_CN = base.FONT_CN
FONT_EN = base.FONT_EN

# candidate-c 的底板口径：近白 → 极浅蓝，蓝色描边，无高光
BG, BG2, FG = "#FFFFFF", "#E7EFFB", "#0B63F6"

# ---------------------------------------------------------------------------
# 图形（24×24）。统一细笔：1.9–2.2。
# ---------------------------------------------------------------------------
# v1 指定版：光圈 5..19（白区 58%），四臂延伸到画布边缘，笔画压到 1.8
# 相对原 C：白区宽 +21%，线宽 −23%，整体外推 +11%
V1 = (
    '<path fill="none" stroke="{{FG}}" stroke-width="1.8" stroke-linecap="round" '
    'stroke-linejoin="round" d="M5 1.4v14.8a2.8 2.8 0 0 0 2.8 2.8h14.8"/>'
    '<path fill="none" stroke="{{FG}}" stroke-width="1.8" stroke-linecap="round" '
    'stroke-linejoin="round" d="M19 22.6V7.8A2.8 2.8 0 0 0 16.2 5H1.4"/>'
)

# v2 大取景框：四角推到 2.8..21.2，臂长 6.0，白区 62%
V2 = (
    '<path fill="none" stroke="{{FG}}" stroke-width="2" stroke-linecap="round" '
    'stroke-linejoin="round" d="M2.8 9V5.8A3 3 0 0 1 5.8 2.8H9'
    'M15 2.8h3.2a3 3 0 0 1 3 3V9'
    'M21.2 15v3.2a3 3 0 0 1-3 3H15'
    'M9 21.2H5.8a3 3 0 0 1-3-3V15"/>'
)

# v3 虚线圈选：周长 4×(14.8−2×4.2)+2π×4.2 ≈ 52，13 个周期 → 段 2.4 / 隔 1.6
# 必须整除，否则首尾错位，转角处会露出一个不均的短划。
V3 = (
    '<rect x="4.6" y="4.6" width="14.8" height="14.8" rx="4.2" fill="none" '
    'stroke="{{FG}}" stroke-width="1.9" stroke-dasharray="2.4 1.6"/>'
)

# v4 三分线取景框：外框 + 井字分格，相机取景的语法
V4 = (
    '<rect x="3.6" y="3.6" width="16.8" height="16.8" rx="4" fill="none" '
    'stroke="{{FG}}" stroke-width="1.7"/>'
    '<path fill="none" stroke="{{FG}}" stroke-width="0.75" '
    'd="M9.2 4.5v15M14.8 4.5v15M4.5 9.2h15M4.5 14.8h15"/>'
)

# v5 偏移双框：两个同尺寸圆角方错位叠放
V5 = (
    '<rect x="3.2" y="3.2" width="13.4" height="13.4" rx="3.2" fill="none" '
    'stroke="{{FG}}" stroke-width="1.9"/>'
    '<rect x="7.4" y="7.4" width="13.4" height="13.4" rx="3.2" fill="none" '
    'stroke="{{FG}}" stroke-width="1.9"/>'
)

# v6 准星：四个刻度 + 中心细环，和"框"系列拉开剪影差异
V6 = (
    '<path fill="none" stroke="{{FG}}" stroke-width="2.1" stroke-linecap="round" '
    'd="M12 2.8v6.2M12 21.2V15M2.8 12h6.2M21.2 12H15"/>'
    '<circle cx="12" cy="12" r="2.7" fill="none" stroke="{{FG}}" stroke-width="1.9"/>'
)

VARIANTS = [
    {"id": "v1", "name": "大光圈裁切", "tag": "指定版 · 白区扩大 / 线更细 / 更靠外",
     "mark": V1, "scale": 0.83},
    {"id": "v2", "name": "大取景框", "tag": "四角外推 / 无中心点", "mark": V2, "scale": 0.86},
    {"id": "v3", "name": "虚线圈选", "tag": "选集语义 / 节奏感", "mark": V3, "scale": 0.86},
    {"id": "v4", "name": "三分线取景", "tag": "相机语法 / 井字分格", "mark": V4, "scale": 0.84},
    {"id": "v5", "name": "偏移双框", "tag": "前后对比 / 错位", "mark": V5, "scale": 0.82},
    {"id": "v6", "name": "准星", "tag": "精准取点 / 非框形剪影", "mark": V6, "scale": 0.84},
]


def render(v: dict, size: int, shape: str = "square", bg2: str | None = BG2) -> Image.Image:
    return base.render(
        {"mark": v["mark"], "fg": FG, "bg": BG, "bg2": bg2,
         "gloss": False, "scale": v["scale"]},
        size, shape)


def build_preview(cols: int = 3) -> Path:
    W, PAD = 1240, 56
    cell_w = (W - PAD * 2 - (cols - 1) * 36) // cols
    icon = 208
    ladder = (64, 48, 32, 24, 16)
    row_h = icon + 92 + 96

    rows = (len(VARIANTS) + cols - 1) // cols
    head = 152
    H = head + row_h * rows + 56

    canvas = Image.new("RGB", (W, H), "#FFFFFF")
    d = ImageDraw.Draw(canvas)
    f_title = ImageFont.truetype(FONT_EN, 26)
    f_sub = ImageFont.truetype(FONT_CN, 13)
    f_name = ImageFont.truetype(FONT_CN, 16)
    f_tag = ImageFont.truetype(FONT_CN, 12)

    d.text((PAD, 44), "Sniptool — line-style variants", font=f_title, fill="#111318")
    d.text((PAD, 82), "沿用候选 C 的浅色底板与蓝色细线；512×512，无文字",
           font=f_sub, fill="#8A8F98")

    for i, v in enumerate(VARIANTS):
        cx = PAD + (i % cols) * (cell_w + 36)
        cy = head + (i // cols) * row_h
        im = render(v, 512).convert("RGB").resize((icon, icon), Image.LANCZOS)
        canvas.paste(im, (cx, cy))
        d.rectangle([cx, cy, cx + icon - 1, cy + icon - 1], outline="#E4E4E7")

        d.text((cx, cy + icon + 16), "%s   %s" % (v["id"].upper(), v["name"]),
               font=f_name, fill="#1B1E24")
        d.text((cx, cy + icon + 42), v["tag"], font=f_tag, fill="#9AA0A8")

        strip_y = cy + icon + 68
        d.rounded_rectangle([cx, strip_y, cx + cell_w, strip_y + 80], radius=10, fill="#F3F3F3")
        lx, lbase = cx + 16, strip_y + 60
        for s in ladder:
            im_s = render(v, s)
            canvas.paste(im_s, (lx, lbase - s), im_s)
            lx += s + 14

    d.text((PAD, H - 40),
           "线条统一 1.9–2.1，底板 #FFFFFF→#E7EFFB；小尺寸条用于判断细线在 16px 是否还成立。",
           font=f_sub, fill="#8A8F98")

    path = OUT / "preview.png"
    canvas.save(path)
    return path


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    for v in VARIANTS:
        render(v, 512, "square").convert("RGB").save(OUT / ("%s.png" % v["id"]), "PNG")
        render(v, 512, "round").save(OUT / ("%s-round.png" % v["id"]), "PNG")
        for s in (16, 24, 32, 48):
            render(v, s).save(OUT / ("%s-%d.png" % (v["id"], s)))
    print("preview:", build_preview())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
