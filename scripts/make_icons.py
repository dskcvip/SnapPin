#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SnapPin 应用图标生成器。

以用户提供的 SVG（橙圆 + S 曲线）为基础重绘为高质感版本：
- 深色圆底（垂直渐变 + 顶部径向高光 + 底部暗角 + 边缘亮环）
- 橙色渐变 S（三段垂直渐变，圆头笔画，带柔和投影）
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

# 圆底渐变（顶 → 底）
BG_TOP = (30, 41, 59)      # #1E293B
BG_BOTTOM = (11, 17, 30)   # #0B1220
# S 渐变（顶 → 底）
S_TOP = (255, 213, 170)    # #FFD5AA
S_MID = (255, 158, 99)     # #FF9E63
S_BOTTOM = (245, 119, 42)  # #F5772A


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


def make_logo(size):
    """渲染 size×size 的高质感图标（RGBA）。"""
    # --- 1. 圆底：垂直渐变 ---
    bg = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(bg)
    for y in range(size):
        t = y / (size - 1)
        # 轻微非线性，让中部过渡更柔
        tt = t * t * (3 - 2 * t)
        d.line([(0, y), (size, y)], fill=lerp(BG_TOP, BG_BOTTOM, tt) + (255,))

    # 圆形裁剪
    mask = Image.new("L", (size, size), 0)
    ImageDraw.Draw(mask).ellipse([0, 0, size - 1, size - 1], fill=255)
    bg.putalpha(mask)

    # --- 2. 质感层：顶部径向高光 + 底部暗角 ---
    gloss = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    gd = ImageDraw.Draw(gloss)
    steps = size // 64
    for i in range(steps):
        t = i / steps
        radius = size * 0.62 * (1 - t) + 1
        alpha = int(26 * (1 - t) ** 2)
        gd.ellipse(
            [size * 0.5 - radius, size * 0.18 - radius * 0.55,
             size * 0.5 + radius, size * 0.18 + radius * 0.55],
            fill=(255, 255, 255, alpha),
        )
    gloss = gloss.filter(ImageFilter.GaussianBlur(size * 0.02))
    bg = Image.alpha_composite(bg, Image.composite(gloss, Image.new("RGBA", (size, size), (0, 0, 0, 0)), mask))

    vignette = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    vd = ImageDraw.Draw(vignette)
    for i in range(steps):
        t = i / steps
        radius = size * 0.55 * (1 - t) + 1
        alpha = int(34 * (1 - t) ** 2)
        vd.ellipse(
            [size * 0.5 - radius, size * 1.02 - radius * 0.5,
             size * 0.5 + radius, size * 1.02 + radius * 0.5],
            fill=(0, 0, 0, alpha),
        )
    vignette = vignette.filter(ImageFilter.GaussianBlur(size * 0.02))
    bg = Image.alpha_composite(bg, Image.composite(vignette, Image.new("RGBA", (size, size), (0, 0, 0, 0)), mask))

    # --- 3. 边缘亮环（1.5px 白 14% + 内侧细暗线） ---
    ring = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    rd = ImageDraw.Draw(ring)
    w = max(1, int(size * 0.006))
    rd.ellipse([w, w, size - 1 - w, size - 1 - w], outline=(255, 255, 255, 38), width=w)
    rd.ellipse([w * 3, w * 3, size - 1 - w * 3, size - 1 - w * 3], outline=(0, 0, 0, 26), width=max(1, w // 2))
    bg = Image.alpha_composite(bg, Image.composite(ring, Image.new("RGBA", (size, size), (0, 0, 0, 0)), mask))

    # --- 4. S：投影 + 渐变笔画 ---
    center = [(x * S, y * S) for (x, y) in build_s_centerline()]
    stroke_w = 108 * S

    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    stroke_polyline(
        shadow,
        [(x, y + 0.012 * size) for (x, y) in center],
        stroke_w,
        lambda x, y: (0, 0, 0, 90),
    )
    shadow = shadow.filter(ImageFilter.GaussianBlur(size * 0.016))
    shadow.putalpha(Image.composite(shadow.getchannel("A"), Image.new("L", (size, size), 0), mask))
    bg = Image.alpha_composite(bg, shadow)

    def s_color(x, y):
        t = max(0.0, min(1.0, y / size))
        if t < 0.5:
            c = lerp(S_TOP, S_MID, t * 2)
        else:
            c = lerp(S_MID, S_BOTTOM, (t - 0.5) * 2)
        return c + (255,)

    stroke_polyline(bg, center, stroke_w, s_color)

    # --- 5. S 顶部细高光（模拟立体感） ---
    hl = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    stroke_polyline(
        hl,
        [(x, y - stroke_w * 0.30) for (x, y) in center],
        stroke_w * 0.30,
        lambda x, y: (255, 255, 255, 70),
    )
    hl = hl.filter(ImageFilter.GaussianBlur(size * 0.004))
    # 高光只在 S 笔画范围内显示：用 S 自身 alpha 做遮罩
    s_mask = bg.getchannel("A")
    hl.putalpha(Image.composite(hl.getchannel("A"), Image.new("L", (size, size), 0), s_mask))
    bg = Image.alpha_composite(bg, hl)

    return bg


# ---------------------------------------------------------------- 输出

PNG_SIZES = [16, 32, 64, 128, 256, 512]


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
        logo.resize((s, s), Image.LANCZOS).save(os.path.join(OUT_DIR, f"{s}x{s}.png"))
    # 主图标 512
    logo.resize((512, 512), Image.LANCZOS).save(os.path.join(OUT_DIR, "icon.png"))
    # Windows Store 系列
    for s, name in [
        (30, "Square30x30Logo.png"), (44, "Square44x44Logo.png"),
        (71, "Square71x71Logo.png"), (89, "Square89x89Logo.png"),
        (107, "Square107x107Logo.png"), (142, "Square142x142Logo.png"),
        (150, "Square150x150Logo.png"), (284, "Square284x284Logo.png"),
        (310, "Square310x310Logo.png"), (256, "StoreLogo.png"),
    ]:
        logo.resize((s, s), Image.LANCZOS).save(os.path.join(OUT_DIR, name))
    logo.resize((256, 256), Image.LANCZOS).save(os.path.join(OUT_DIR, "128x128@2x.png"))

    # ICO：内嵌多尺寸
    ico_sizes = [16, 32, 48, 64, 128, 256]
    ico_imgs = [logo.resize((s, s), Image.LANCZOS) for s in ico_sizes]
    ico_imgs[0].save(
        os.path.join(OUT_DIR, "icon.ico"),
        format="ICO",
        sizes=[(s, s) for s in ico_sizes],
        append_images=ico_imgs[1:],
    )

    # ICNS（macOS）
    icns_imgs = {s: logo.resize((s, s), Image.LANCZOS) for s in [16, 32, 128, 256, 512]}
    icns_imgs[1024] = logo.resize((1024, 1024), Image.LANCZOS)
    write_icns(icns_imgs, os.path.join(OUT_DIR, "icon.icns"))

    print("icons written to", OUT_DIR)


if __name__ == "__main__":
    main()
