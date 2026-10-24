#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SnapPin 应用图标生成器。

风格参照用户桌面那批应用图标（PixPin 的 P、WorkBuddy 的 W、语雀的鸟）：
**圆角方形底 + 品牌色 + 纯白简洁字形**，扁平现代，去掉了旧的
"深色圆底 + 橙色渐变 S + 高光/暗角"那套拟物做法。

- 底：圆角矩形（半径 28%），品牌蓝垂直微渐变（顶亮底深）
- 字：白色 S，圆头笔画；小尺寸加粗（光学补偿）
- 2x 超采样抗锯齿，输出 PNG 全家 + icon.ico + icon.icns

用法：python scripts/make_icons.py
"""
import math
import os
import struct

from PIL import Image, ImageDraw, ImageFilter

# ---------------------------------------------------------------- 参数

OUT_DIR = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "src-tauri", "icons")
BASE = 2048          # 超采样基准尺寸（2x of 1024）
S = BASE / 1024.0    # 设计坐标(1024) → 像素 的比例

# 品牌蓝（与 config.themeColor 默认值 #12A5D8 一致）
BRAND = (0x12, 0xA5, 0xD8)
BRAND_LIGHT = (0x45, 0xC6, 0xF0)   # 顶部提亮
# 字形：纯白（扁平风格，不再做渐变）
GLYPH = (255, 255, 255)


# ---------------------------------------------------------------- 贝塞尔工具

def cubic(p0, p1, p2, p3, n=48):
    """三次贝塞尔采样为折线点列。"""
    pts = []
    for i in range(n + 1):
        t = i / n
        mt = 1 - t
        a = mt ** 3
        b = 3 * mt * mt * t
        c = 3 * mt * t * t
        d = t ** 3
        x = a * p0[0] + b * p1[0] + c * p2[0] + d * p3[0]
        y = a * p0[1] + b * p1[1] + c * p2[1] + d * p3[1]
        pts.append((x, y))
    return pts


def build_s_centerline():
    """S 字母中心线：4 段三次贝塞尔，坐标为 1024 设计空间。"""
    segs = [
        # 上弧：右上 → 顶 → 左上
        ((700, 300), (560, 210), (420, 215), (375, 330)),
        # 中过渡：左上 → 中 → 中右
        ((375, 330), (340, 430), (480, 470), (600, 500)),
        # 中下：中右 → 右下
        ((600, 500), (680, 520), (690, 590), (648, 655)),
        # 下弧：右下 → 底 → 左下
        ((648, 655), (600, 730), (470, 745), (360, 700)),
    ]
    pts = []
    for i, (p0, p1, p2, p3) in enumerate(segs):
        seg_pts = cubic(p0, p1, p2, p3)
        pts.extend(seg_pts if i == 0 else seg_pts[1:])
    return pts


# ---------------------------------------------------------------- 绘制原语

def stroke_polyline(img, pts, width_px, color_fn):
    """沿折线画圆头粗线（与 core-algo::draw::draw_thick_line 同算法）。
    color_fn(x, y) -> RGBA，支持沿线渐变。"""
    px = img.load()
    r = width_px / 2.0
    r2 = (r + 0.6) ** 2
    for i in range(len(pts) - 1):
        (x0, y0), (x1, y1) = pts[i], pts[i + 1]
        dx, dy = x1 - x0, y1 - y0
        steps = max(1, int(math.hypot(dx, dy) / (r * 0.5)))
        for s in range(steps + 1):
            t = s / steps
            cx, cy = x0 + dx * t, y0 + dy * t
            x0i, x1i = int(cx - r - 1), int(cx + r + 1)
            y0i, y1i = int(cy - r - 1), int(cy + r + 1)
            for yy in range(max(0, y0i), min(img.height, y1i + 1)):
                for xx in range(max(0, x0i), min(img.width, x1i + 1)):
                    ddx, ddy = xx - cx, yy - cy
                    if ddx * ddx + ddy * ddy <= r2:
                        px[xx, yy] = color_fn(xx, yy)


def lerp(a, b, t):
    return tuple(int(round(a[i] + (b[i] - a[i]) * t)) for i in range(3))


def s_metrics(size):
    """按最终像素尺寸给 S 的笔画比例与字形缩放（光学补偿）。

    小尺寸的问题不是"没超采样"，而是**白笔画太细**：抗锯齿把 1~2px 的笔画
    和背景混在一起，视觉上就糊了。这里直接加粗 + 略放大字形，
    保证 16px 下 S 的转折仍能分辨。
    """
    if size <= 20:
        return 182, 1.10
    if size <= 26:
        return 162, 1.06
    if size <= 40:
        return 142, 1.02
    return 104, 1.0


def rounded_mask(size, radius_ratio=0.28):
    """圆角方形蒙版（squircle 观感，用普通圆角矩形近似）。"""
    mask = Image.new("L", (size, size), 0)
    r = size * radius_ratio
    ImageDraw.Draw(mask).rounded_rectangle(
        [0, 0, size - 1, size - 1], radius=r, fill=255
    )
    return mask


def make_logo(size, detail=True):
    """渲染 size×size 图标（RGBA）：圆角方底 + 白色 S。"""
    bg = Image.new("RGBA", (size, size), (0, 0, 0, 0))

    # --- 1. 圆角底：垂直微渐变（顶亮底深，扁平但有层次） ---
    d = ImageDraw.Draw(bg)
    for y in range(size):
        t = y / max(1, size - 1)
        c = lerp(BRAND_LIGHT, BRAND, min(1.0, t * 1.25))
        d.line([(0, y), (size, y)], fill=c + (255,))
    mask = rounded_mask(size)
    bg.putalpha(mask)

    # --- 2. 白色 S ---
    #
    # 光学补偿（用户 2026-10-25 提醒"注意清晰度"）：1024 基准下 104 的笔画比例
    # 缩到 16px 只有 1.6px，抗锯齿会把笔画吃成灰糊 —— 小尺寸必须同时
    #   a) 加粗笔画  b) 把字形整体放大一点，让"白/蓝"面积比站得住
    # 判定按**最终像素尺寸**分档，而不是看 detail 标志。
    sc = size / 1024.0
    stroke_ratio, glyph_scale = s_metrics(size)
    cx0 = cy0 = 512.0
    center = [
        ((x - cx0) * glyph_scale * sc + size / 2.0,
         (y - cy0) * glyph_scale * sc + size / 2.0)
        for (x, y) in build_s_centerline()
    ]
    stroke_w = stroke_ratio * glyph_scale * sc
    stroke_polyline(bg, center, stroke_w, lambda x, y: GLYPH + (255,))

    # --- 3. 内描边：小尺寸靠一圈浅白勾边撑清晰度 ---
    if not detail:
        ring = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        w = max(1, int(size * 0.012))
        ImageDraw.Draw(ring).rounded_rectangle(
            [w, w, size - 1 - w, size - 1 - w],
            radius=size * 0.28,
            outline=(255, 255, 255, 60),
            width=w,
        )
        bg = Image.alpha_composite(bg, ring)

    return bg


# ---------------------------------------------------------------- 输出

PNG_SIZES = [16, 32, 64, 128, 256, 512]

SMALL_MAX = 32  # ≤32px 走「简化版」光学尺寸（去阴影/高光、加粗笔画、提亮）


def render(logo, size):
    """按尺寸选择渲染路径：小尺寸单独渲染简化版，避免从 2048 直接缩成糊团。"""
    if size <= SMALL_MAX:
        return make_logo(size * 8, detail=False).resize((size, size), Image.LANCZOS)
    return logo.resize((size, size), Image.LANCZOS)


def write_icns(images_by_size, path):
    """手写 icns 容器（Apple Icon Image format）：
    magic 'icns' + total_size + entries(OSType + len + PNG data)。"""
    ostypes = {
        16: b"is32",   # 16x32 小
        32: b"il32",   # 32x32 小
        64: b"ih32",   # 48x48? 实际 64 用 ic07 系列
        128: b"ic07",  # 128x128
        256: b"ic08",  # 128x128@2x
        512: b"ic09",  # 256x256@2x
        1024: b"ic10", # 512x512@2x
    }
    entries = []
    for size, img in images_by_size.items():
        if size not in ostypes:
            continue
        import io
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        data = buf.getvalue()
        entries.append((ostypes[size], data))
    body = b"".join(t + struct.pack(">I", len(d) + 8) + d for t, d in entries)
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", len(body) + 8) + body)


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    logo = make_logo(BASE)

    # PNG 全家
    for s in PNG_SIZES:
        render(logo, s).save(os.path.join(OUT_DIR, f"{s}x{s}.png"))
    # 主图标 512
    render(logo, 512).save(os.path.join(OUT_DIR, "icon.png"))
    # Windows Store 系列
    for s, name in [
        (30, "Square30x30Logo.png"), (44, "Square44x44Logo.png"),
        (71, "Square71x71Logo.png"), (89, "Square89x89Logo.png"),
        (107, "Square107x107Logo.png"), (142, "Square142x142Logo.png"),
        (150, "Square150x150Logo.png"), (284, "Square284x284Logo.png"),
        (310, "Square310x310Logo.png"), (256, "StoreLogo.png"),
    ]:
        render(logo, s).save(os.path.join(OUT_DIR, name))
    render(logo, 256).save(os.path.join(OUT_DIR, "128x128@2x.png"))

    # ICO：内嵌多尺寸
    # 注意：Windows Shell 只对 256x256 官方支持 PNG 压缩帧，小于 256 的 PNG 帧
    # 会被忽略，Shell 只能拿相邻尺寸缩放 → 资源管理器里图标发虚/模糊。
    # 因此这里必须用 bitmap_format="bmp"（DIB 帧），并补齐 20/24/96 等 DPI 常用档位。
    # 另：Pillow 以「基准图」尺寸为上限，超过基准尺寸的档位会被静默丢弃，
    # 所以基准图用最大的 256，其余走 append_images。
    ico_sizes = [16, 20, 24, 32, 48, 64, 96, 128, 256]
    ico_imgs = [render(logo, s) for s in ico_sizes]
    ico_imgs[-1].save(
        os.path.join(OUT_DIR, "icon.ico"),
        format="ICO",
        sizes=[(s, s) for s in ico_sizes],
        append_images=ico_imgs[:-1],
        bitmap_format="bmp",
    )

    # ICNS（macOS）
    icns_imgs = {s: render(logo, s) for s in [16, 32, 128, 256, 512]}
    icns_imgs[1024] = logo.resize((1024, 1024), Image.LANCZOS)
    write_icns(icns_imgs, os.path.join(OUT_DIR, "icon.icns"))

    print("icons written to", OUT_DIR)


if __name__ == "__main__":
    main()
