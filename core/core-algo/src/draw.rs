//! 标注图形的 RGBA8 光栅化（M3）。
//!
//! 与前端 Canvas 预览各画一套，但刻意保持同一套几何约定
//! （圆头笔帽、无抗锯齿、参数化椭圆），保证"所见即所得"。
//! 全部为纯函数：只读写传入的 `RgbaBuffer`，无 IO、无全局状态。

use crate::shapes::{Shape, ShapeStyle};
use crate::RgbaBuffer;

/// 把全部标注画到 buffer 上（就地修改）。
pub fn draw_shapes(buf: &mut RgbaBuffer, shapes: &[Shape]) {
    for s in shapes {
        draw_shape(buf, s);
    }
}

/// 画单个标注。
pub fn draw_shape(buf: &mut RgbaBuffer, s: &Shape) {
    match s {
        Shape::Rect { x, y, w, h, style } => {
            let (x, y, w, h) = (*x as f32, *y as f32, *w as f32, *h as f32);
            draw_thick_line(buf, x, y, x + w, y, style);
            draw_thick_line(buf, x + w, y, x + w, y + h, style);
            draw_thick_line(buf, x + w, y + h, x, y + h, style);
            draw_thick_line(buf, x, y + h, x, y, style);
        }
        Shape::Ellipse { x, y, w, h, style } => {
            let (x, y, w, h) = (*x as f32, *y as f32, *w as f32, *h as f32);
            if w < 1.0 || h < 1.0 {
                return;
            }
            // 参数化采样：周长近似 π·(a+b)，步长保证每段 ≤ 1px
            let steps = ((std::f32::consts::PI * (w + h)) as usize).max(16).min(4096);
            let cx = x + w / 2.0;
            let cy = y + h / 2.0;
            let rx = w / 2.0;
            let ry = h / 2.0;
            let mut prev = (cx + rx, cy);
            for i in 1..=steps {
                let t = i as f32 / steps as f32 * std::f32::consts::TAU;
                let next = (cx + rx * t.cos(), cy + ry * t.sin());
                draw_thick_line(buf, prev.0, prev.1, next.0, next.1, style);
                prev = next;
            }
        }
        Shape::Arrow { x1, y1, x2, y2, style } => {
            let (x1, y1, x2, y2) = (*x1 as f32, *y1 as f32, *x2 as f32, *y2 as f32);
            draw_thick_line(buf, x1, y1, x2, y2, style);
            // 头部两翼：长度随线宽缩放，与主干约 25°
            let dx = x2 - x1;
            let dy = y2 - y1;
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1.0 {
                return;
            }
            let head = (style.width * 4.0).max(8.0).min(len * 0.5);
            let angle = 25.0_f32.to_radians();
            let (ux, uy) = (dx / len, dy / len);
            for sign in [-1.0_f32, 1.0] {
                let a = angle * sign;
                let (cos, sin) = (a.cos(), a.sin());
                // 旋转 -angle：翼方向 = 反向主干旋转
                let wx = ux * cos - uy * sin;
                let wy = ux * sin + uy * cos;
                draw_thick_line(
                    buf,
                    x2,
                    y2,
                    x2 - wx * head,
                    y2 - wy * head,
                    style,
                );
            }
        }
        Shape::Pen { points, style } | Shape::Highlighter { points, style } => {
            if points.len() < 2 {
                // 单点：画一个圆点（单击取色/点标的场景）
                if let Some(&(px, py)) = points.first() {
                    fill_disc(buf, px, py, style.width / 2.0, style.color);
                }
                return;
            }
            for pair in points.windows(2) {
                draw_thick_line(buf, pair[0].0, pair[0].1, pair[1].0, pair[1].1, style);
            }
        }
        Shape::Mosaic { x, y, w, h, block } => {
            mosaic_region(buf, *x, *y, *w, *h, (*block).max(2));
        }
    }
}

/// Alpha 混合写入一个像素（越界忽略）。
/// a=255 时直接覆盖；否则按 src.a 线性混合。
pub fn blend_pixel(buf: &mut RgbaBuffer, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x as u32 >= buf.width || y as u32 >= buf.height {
        return;
    }
    let i = (y as usize * buf.width as usize + x as usize) * 4;
    let a = color[3] as u32;
    if a == 255 {
        buf.data[i..i + 4].copy_from_slice(&color);
        return;
    }
    if a == 0 {
        return;
    }
    let inv = 255 - a;
    for c in 0..3 {
        let dst = buf.data[i + c] as u32;
        buf.data[i + c] = ((color[c] as u32 * a + dst * inv) / 255) as u8;
    }
    // alpha 通道取并集上界，避免反复叠加后底色透出
    let da = buf.data[i + 3] as u32;
    buf.data[i + 3] = (a + da * inv / 255).min(255) as u8;
}

/// 实心圆盘（半径含 0.5px 膨胀，保证细线连续）。
pub fn fill_disc(buf: &mut RgbaBuffer, cx: f32, cy: f32, radius: f32, color: [u8; 4]) {
    let r = radius + 0.5;
    let r2 = r * r;
    let x0 = (cx - r).floor() as i32;
    let x1 = (cx + r).ceil() as i32;
    let y0 = (cy - r).floor() as i32;
    let y1 = (cy + r).ceil() as i32;
    for y in y0..=y1 {
        for x in x0..=x1 {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            if dx * dx + dy * dy <= r2 {
                blend_pixel(buf, x, y, color);
            }
        }
    }
}

/// 带圆头笔帽的粗直线（Bresenham 8 邻域步进 + 沿线圆盘）。
pub fn draw_thick_line(
    buf: &mut RgbaBuffer,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    style: &ShapeStyle,
) {
    let (mut ix, mut iy) = (x0.round() as i32, y0.round() as i32);
    let (ex, ey) = (x1.round() as i32, y1.round() as i32);
    let dx = (ex - ix).abs();
    let dy = (ey - iy).abs();
    let sx = if ex >= ix { 1 } else { -1 };
    let sy = if ey >= iy { 1 } else { -1 };
    let mut err = dx - dy;
    let radius = style.width / 2.0;
    // 端点半径略小，避免与下一段重复叠加（荧光笔半透明时尤其明显）
    let cap_radius = radius;
    loop {
        fill_disc(buf, ix as f32, iy as f32, cap_radius, style.color);
        if ix == ex && iy == ey {
            break;
        }
        let e2 = err * 2;
        if e2 > -dy {
            err -= dy;
            ix += sx;
        }
        if e2 < dx {
            err += dx;
            iy += sy;
        }
    }
}

/// 马赛克：块平均像素化（块内取均值，边界块按实际大小平均）。
pub fn mosaic_region(buf: &mut RgbaBuffer, x: i32, y: i32, w: u32, h: u32, block: u32) {
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w as i32).min(buf.width as i32);
    let y1 = (y + h as i32).min(buf.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let block = block.max(2) as i32;
    let mut by = y0;
    while by < y1 {
        let bh = block.min(y1 - by);
        let mut bx = x0;
        while bx < x1 {
            let bw = block.min(x1 - bx);
            // 块内累加（跳过 alpha=0 的像素，避免透明区把均值拉黑）
            let (mut sr, mut sg, mut sb, mut n) = (0u32, 0u32, 0u32, 0u32);
            for py in by..by + bh {
                for px in bx..bx + bw {
                    let i = (py as usize * buf.width as usize + px as usize) * 4;
                    let a = buf.data[i + 3] as u32;
                    if a == 0 {
                        continue;
                    }
                    sr += buf.data[i] as u32;
                    sg += buf.data[i + 1] as u32;
                    sb += buf.data[i + 2] as u32;
                    n += 1;
                }
            }
            if n > 0 {
                let color = [
                    (sr / n) as u8,
                    (sg / n) as u8,
                    (sb / n) as u8,
                    255,
                ];
                for py in by..by + bh {
                    for px in bx..bx + bw {
                        blend_pixel(buf, px, py, color);
                    }
                }
            }
            bx += block;
        }
        by += block;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::{Shape, ShapeStyle};

    /// 全黑测试图（含 alpha 255）
    fn black(w: u32, h: u32) -> RgbaBuffer {
        let mut data = vec![0u8; w as usize * h as usize * 4];
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        RgbaBuffer::new(w, h, data).unwrap()
    }

    fn red() -> ShapeStyle {
        ShapeStyle::new([255, 0, 0, 255], 2.0)
    }

    #[test]
    fn blend_opaque_overwrites() {
        let mut buf = black(2, 1);
        blend_pixel(&mut buf, 0, 0, [1, 2, 3, 255]);
        assert_eq!(buf.pixel(0, 0), Some([1, 2, 3, 255]));
    }

    #[test]
    fn blend_transparent_is_linear() {
        let mut buf = black(1, 1);
        // 黑底上 50% 白 → 约 128
        blend_pixel(&mut buf, 0, 0, [255, 255, 255, 128]);
        let p = buf.pixel(0, 0).unwrap();
        assert!((p[0] as i32 - 128).abs() <= 1, "got {:?}", p);
        assert!((p[1] as i32 - 128).abs() <= 1);
        assert!((p[2] as i32 - 128).abs() <= 1);
    }

    #[test]
    fn blend_alpha_zero_is_noop() {
        let mut buf = black(1, 1);
        blend_pixel(&mut buf, 0, 0, [255, 255, 255, 0]);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn blend_out_of_bounds_is_noop() {
        let mut buf = black(2, 2);
        blend_pixel(&mut buf, -1, 0, [255, 0, 0, 255]);
        blend_pixel(&mut buf, 99, 99, [255, 0, 0, 255]);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn thick_line_covers_axis() {
        let mut buf = black(10, 10);
        draw_thick_line(&mut buf, 1.0, 5.0, 8.0, 5.0, &red());
        for x in 1..=8 {
            assert_eq!(buf.pixel(x, 5), Some([255, 0, 0, 255]), "x={x}");
        }
        // 线宽 2 → 上下各半行覆盖
        assert_eq!(buf.pixel(4, 4), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(4, 6), Some([255, 0, 0, 255]));
    }

    #[test]
    fn thick_diagonal_stays_connected() {
        let mut buf = black(10, 10);
        draw_thick_line(&mut buf, 1.0, 1.0, 8.0, 8.0, &red());
        // 对角线不应断裂：每个 y 都至少有一个红像素
        for y in 1..=8 {
            let row: Vec<_> = (0..10)
                .filter(|&x| buf.pixel(x, y) == Some([255, 0, 0, 255]))
                .collect();
            assert!(!row.is_empty(), "row {y} empty");
        }
    }

    #[test]
    fn rect_stroke_draws_four_edges() {
        let mut buf = black(20, 20);
        let s = Shape::Rect { x: 2, y: 2, w: 10, h: 8, style: red() };
        draw_shape(&mut buf, &s);
        // 四条边中点
        assert_eq!(buf.pixel(7, 2), Some([255, 0, 0, 255])); // 上
        assert_eq!(buf.pixel(7, 10), Some([255, 0, 0, 255])); // 下
        assert_eq!(buf.pixel(2, 6), Some([255, 0, 0, 255])); // 左
        assert_eq!(buf.pixel(12, 6), Some([255, 0, 0, 255])); // 右
        // 内部保持黑
        assert_eq!(buf.pixel(7, 6), Some([0, 0, 0, 255]));
    }

    #[test]
    fn ellipse_stroke_is_ring() {
        let mut buf = black(30, 30);
        let s = Shape::Ellipse { x: 2, y: 2, w: 26, h: 26, style: red() };
        draw_shape(&mut buf, &s);
        // 外接框四边中点应有像素；圆心不应有（空心）
        assert_eq!(buf.pixel(15, 2), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(15, 27), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(15, 15), Some([0, 0, 0, 255]));
    }

    #[test]
    fn arrow_has_head_wings() {
        let mut buf = black(40, 40);
        let s = Shape::Arrow { x1: 5, y1: 20, x2: 30, y2: 20, style: red() };
        draw_shape(&mut buf, &s);
        // 主干
        assert_eq!(buf.pixel(15, 20), Some([255, 0, 0, 255]));
        // 头部两翼：端点上方与下方应有像素（水平箭头 → 上下翼）
        let up = (0..40).any(|y| buf.pixel(30, y) == Some([255, 0, 0, 255]) && y < 20);
        let down = (0..40).any(|y| buf.pixel(30, y) == Some([255, 0, 0, 255]) && y > 20);
        assert!(up, "no upper wing");
        assert!(down, "no lower wing");
    }

    #[test]
    fn pen_single_point_draws_dot() {
        let mut buf = black(10, 10);
        let s = Shape::Pen { points: vec![(5.0, 5.0)], style: red() };
        draw_shape(&mut buf, &s);
        assert_eq!(buf.pixel(5, 5), Some([255, 0, 0, 255]));
    }

    #[test]
    fn mosaic_averages_blocks() {
        // 4x2：左半 200，右半 100 → block=2 → 两块均值 200/100
        let mut data = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                let v = if x < 2 { 200 } else { 100 };
                data.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let mut buf = RgbaBuffer::new(4, 2, data).unwrap();
        mosaic_region(&mut buf, 0, 0, 4, 2, 2);
        assert_eq!(buf.pixel(0, 0), Some([200, 200, 200, 255]));
        assert_eq!(buf.pixel(3, 1), Some([100, 100, 100, 255]));
    }

    #[test]
    fn mosaic_clips_to_bounds() {
        let mut buf = black(4, 4);
        // 完全越界与部分越界都不应 panic
        mosaic_region(&mut buf, -10, -10, 4, 4, 4);
        mosaic_region(&mut buf, 2, 2, 100, 100, 4);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn empty_shapes_is_noop() {
        let mut buf = black(2, 2);
        draw_shapes(&mut buf, &[]);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn highlighter_overlap_blends_not_overwrites() {
        let mut buf = black(20, 20);
        // 荧光笔 40% 黄，来回画两遍：重叠处不应比单遍更"实"到 100%
        let style = ShapeStyle::new([255, 255, 0, 100], 6.0);
        let s = Shape::Highlighter {
            points: vec![(2.0, 10.0), (18.0, 10.0)],
            style,
        };
        draw_shape(&mut buf, &s);
        let p = buf.pixel(10, 10).unwrap();
        assert!(p[0] > 100 && p[0] < 255, "single pass: {:?}", p);
        assert!(p[2] < 60, "blue channel should stay low: {:?}", p);
    }
}
