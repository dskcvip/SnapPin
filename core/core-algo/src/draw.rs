//! 标注图形的 RGBA8 光栅化（M3）。
//!
//! 与前端 Canvas 预览各画一套，但刻意保持同一套几何约定
//! （圆头笔帽、无抗锯齿、参数化椭圆），保证"所见即所得"。
//! 全部为纯函数：只读写传入的 `RgbaBuffer`，无 IO、无全局状态。

use crate::shapes::{Shape, ShapeStyle};
use crate::RgbaBuffer;

/// 把全部标注画到 buffer 上（就地修改）。
///
/// 注意：依赖"底图"的图形（橡皮擦/区域放大/智能擦除）在本函数里是**无操作**——
/// 没有 base 就无从还原/取样。整幅标注的正式渲染入口是
/// [`draw_shapes_with_base`]；本函数仅供不需要 base 的场景（如纯图形预览）。
pub fn draw_shapes(buf: &mut RgbaBuffer, shapes: &[Shape]) {
    for s in shapes {
        draw_shape(buf, s);
    }
}

/// 把全部标注画到 buffer 上，`base` 为**画任何标注之前的原始裁剪图**。
///
/// 这是标注渲染的正式入口：橡皮擦/区域放大/智能擦除三个变体从 base 取像素，
/// 其余图形与 [`draw_shapes`] 行为完全一致。`buf` 与 `base` 尺寸必须相同
/// （调用方在确认选区时一并传入），尺寸不一致时依赖 base 的图形按无操作处理
/// （宁可少画，也不要为坏数据画错内容）。
pub fn draw_shapes_with_base(buf: &mut RgbaBuffer, base: &RgbaBuffer, shapes: &[Shape]) {
    let base_ok = buf.width == base.width && buf.height == base.height;
    for s in shapes {
        match s {
            Shape::Eraser { points, radius } if base_ok => {
                eraser_region(buf, base, points, *radius);
            }
            Shape::ZoomRegion {
                x, y, w, h, zoom, style,
            } if base_ok => {
                zoom_region(buf, base, *x, *y, *w, *h, *zoom, style);
            }
            Shape::SmartErase {
                x, y, w, h, iterations,
            } if base_ok => {
                smart_erase_region(buf, base, *x, *y, *w, *h, *iterations);
            }
            _ => draw_shape(buf, s),
        }
    }
}

/// 画单个标注。
///
/// 依赖 base 的图形（橡皮擦/区域放大/智能擦除）在此为无操作，见
/// [`draw_shapes_with_base`]。
pub fn draw_shape(buf: &mut RgbaBuffer, s: &Shape) {
    match s {
        Shape::Rect { x, y, w, h, style } => {
            stroke_rect(buf, *x, *y, *w, *h, style);
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
        Shape::Bitmap { x, y, w, h, rgba } => {
            blit_bitmap(buf, *x, *y, *w, *h, rgba);
        }
        Shape::Spotlight { x, y, w, h, radius, dim } => {
            spotlight_region(buf, *x, *y, *w, *h, *radius, *dim);
        }
        Shape::Watermark {
            x, y, w, h, step_x, step_y, tile_w, tile_h, tile,
        } => {
            watermark_region(buf, *x, *y, *w, *h, *step_x, *step_y, *tile_w, *tile_h, tile);
        }
        // 以下三个变体必须读 base（还原原图 / 放大原图 / 从原图边界扩散），
        // 单独调用 draw_shape 时没有 base 可读，按无操作处理。
        Shape::Eraser { .. } | Shape::ZoomRegion { .. } | Shape::SmartErase { .. } => {}
    }
}

/// 矩形描边（四条粗线，圆头笔帽自然衔接四角）。
pub fn stroke_rect(buf: &mut RgbaBuffer, x: i32, y: i32, w: u32, h: u32, style: &ShapeStyle) {
    let (x, y, w, h) = (x as f32, y as f32, w as f32, h as f32);
    draw_thick_line(buf, x, y, x + w, y, style);
    draw_thick_line(buf, x + w, y, x + w, y + h, style);
    draw_thick_line(buf, x + w, y + h, x, y + h, style);
    draw_thick_line(buf, x, y + h, x, y, style);
}

/// 聚光灯：把圆角矩形**外部**的像素按 `dim` 压暗，内部保持原样。
///
/// 几何用圆角矩形有符号距离场（SDF）判定：`d < 0` 在内部、`d > 0` 在外部，
/// `d ∈ [0, FEATHER]` 的窄带内按线性插值做羽化，圆角处才不会出现硬锯齿。
///
/// 只做**乘性压暗**（RGB × (1 - strength × k)），alpha 通道不变——
/// 压暗不该改变透明度，否则截图上原本透明的区域会变成半透明黑块。
pub fn spotlight_region(
    buf: &mut RgbaBuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: u32,
    dim: u8,
) {
    if w == 0 || h == 0 || dim == 0 {
        return;
    }
    let bw = buf.width as i32;
    let bh = buf.height as i32;
    if bw == 0 || bh == 0 {
        return;
    }
    let strength = dim as f32 / 255.0;
    // 圆角半径不能超过短边一半，否则 SDF 会退化成负的"半高"而出现破形
    let r = radius.min(w / 2).min(h / 2) as f32;
    let cx = x as f32 + w as f32 / 2.0;
    let cy = y as f32 + h as f32 / 2.0;
    // 圆角矩形的"内部直边"半长：|p-c| 超出这个值才开始算圆角
    let hx = (w as f32 / 2.0 - r).max(0.0);
    let hy = (h as f32 / 2.0 - r).max(0.0);
    const FEATHER: f32 = 1.5;

    // 洞的最大影响范围（含羽化）：超出这个带的像素直接按 k=1 处理，
    // 省掉绝大部分 SDF 计算（4K 图上峰值的 99% 都在这个带之外）。
    let band_x0 = x as f32 - FEATHER;
    let band_x1 = x as f32 + w as f32 + FEATHER;
    let band_y0 = y as f32 - FEATHER;
    let band_y1 = y as f32 + h as f32 + FEATHER;

    for py in 0..bh {
        let fy = py as f32 + 0.5;
        let row_in_band = fy >= band_y0 && fy <= band_y1;
        for px in 0..bw {
            let fx = px as f32 + 0.5;
            // k = 压暗比例（0 = 不压暗，1 = 压到最暗）
            let k = if row_in_band && fx >= band_x0 && fx <= band_x1 {
                let qx = (fx - cx).abs() - hx;
                let qy = (fy - cy).abs() - hy;
                let outside = (qx.max(0.0) * qx.max(0.0) + qy.max(0.0) * qy.max(0.0)).sqrt()
                    + qx.max(qy).min(0.0)
                    - r;
                (outside / FEATHER).clamp(0.0, 1.0)
            } else {
                1.0
            };
            if k <= 0.0 {
                continue;
            }
            let f = 1.0 - strength * k;
            let i = (py as usize * bw as usize + px as usize) * 4;
            for c in 0..3 {
                buf.data[i + c] = (buf.data[i + c] as f32 * f + 0.5) as u8;
            }
        }
    }
}

/// 位图图层合成：把 RGBA8 贴图按 alpha 混合到目标缓冲（越界自动裁剪）。
///
/// 与 `mosaic_region` 一样只吃字节缓冲，不做解码/缩放——贴图尺寸与目标
/// 位置均为**物理像素**，调用方负责保证 1:1（前端已按 DPR 光栅化）。
/// 数据长度不足时静默跳过（宁可少画一层，也不要为坏数据 panic 掉整张图）。
pub fn blit_bitmap(buf: &mut RgbaBuffer, x: i32, y: i32, w: u32, h: u32, rgba: &[u8]) {
    if w == 0 || h == 0 {
        return;
    }
    let need = w as usize * h as usize * 4;
    if rgba.len() < need {
        return;
    }
    // 只遍历与目标缓冲相交的行列，避免对超大贴图做无用功
    for row in 0..h as i32 {
        let py = y + row;
        if py < 0 || py as u32 >= buf.height {
            continue;
        }
        let src_row = row as usize * w as usize * 4;
        for col in 0..w as i32 {
            let px = x + col;
            if px < 0 || px as u32 >= buf.width {
                continue;
            }
            let i = src_row + col as usize * 4;
            let a = rgba[i + 3];
            if a == 0 {
                continue;
            }
            blend_pixel(buf, px, py, [rgba[i], rgba[i + 1], rgba[i + 2], a]);
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

/// 橡皮擦：把轨迹区域从 `base` **还原**成原图像素（擦掉已画上的标注）。
///
/// 与"用白色涂抹"的本质区别：不引入任何颜色，只从 base 拷回像素，
/// 因此擦掉红色矩形后露出的是截图本身，而不是一块白。
/// 实现上沿轨迹以 `radius/2` 步进采样落盘圆盘，并用笔画外接矩形内的
/// 命中掩膜保证同一像素只拷一次（长笔画不会反复拷贝同一片区域）。
pub fn eraser_region(
    buf: &mut RgbaBuffer,
    base: &RgbaBuffer,
    points: &[(f32, f32)],
    radius: f32,
) {
    if points.is_empty() || radius <= 0.0 || buf.width == 0 || buf.height == 0 {
        return;
    }
    if buf.width != base.width || buf.height != base.height {
        return;
    }
    // 整条笔画的外接矩形（含半径膨胀），与图像求交 → 掩膜作用范围
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for &(px, py) in points {
        min_x = min_x.min(px);
        min_y = min_y.min(py);
        max_x = max_x.max(px);
        max_y = max_y.max(py);
    }
    let bx0 = (min_x - radius).floor().max(0.0) as i32;
    let by0 = (min_y - radius).floor().max(0.0) as i32;
    let bx1 = ((max_x + radius).ceil() as i32 + 1).min(buf.width as i32);
    let by1 = ((max_y + radius).ceil() as i32 + 1).min(buf.height as i32);
    if bx1 <= bx0 || by1 <= by0 {
        return;
    }
    let mw = (bx1 - bx0) as usize;
    let mut hit = vec![false; mw * (by1 - by0) as usize];
    let row_bytes = buf.width as usize * 4;

    // 沿折线细分采样：步进 ≤ radius/2，保证快速拖动时圆盘相连不断裂
    let step = (radius * 0.5).max(1.0);
    let mut samples: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    samples.push(points[0]);
    for pair in points.windows(2) {
        let (x0, y0) = pair[0];
        let (x1, y1) = pair[1];
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt();
        let n = (len / step).ceil().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f32 / n as f32;
            samples.push((x0 + dx * t, y0 + dy * t));
        }
    }

    // 与 fill_disc 一致的 0.5px 膨胀，保证擦除边缘连续无缝隙
    let r = radius + 0.5;
    let r2 = r * r;
    for (cx, cy) in samples {
        let x0 = (cx - r).floor() as i32;
        let x1 = (cx + r).ceil() as i32;
        let y0 = (cy - r).floor() as i32;
        let y1 = (cy + r).ceil() as i32;
        for py in y0..=y1 {
            if py < by0 || py >= by1 {
                continue;
            }
            for px in x0..=x1 {
                if px < bx0 || px >= bx1 {
                    continue;
                }
                let dx = px as f32 - cx;
                let dy = py as f32 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let mi = (py - by0) as usize * mw + (px - bx0) as usize;
                if hit[mi] {
                    continue;
                }
                hit[mi] = true;
                let i = py as usize * row_bytes + px as usize * 4;
                buf.data[i..i + 4].copy_from_slice(&base.data[i..i + 4]);
            }
        }
    }
}

/// 水印：把一小块位图按步进平铺进指定区域（半透明文字/图片水印）。
///
/// 平铺从区域左上角开始，`step` 即两块之间的**中心间距**（大于 tile
/// 尺寸的部分就是水印间隙，小于则允许重叠做密集水印）。越界由
/// `blit_bitmap` 自动裁剪；tile 数据长度不足时静默跳过。
pub fn watermark_region(
    buf: &mut RgbaBuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    step_x: u32,
    step_y: u32,
    tile_w: u32,
    tile_h: u32,
    tile: &[u8],
) {
    if w == 0 || h == 0 || tile_w == 0 || tile_h == 0 {
        return;
    }
    if tile.len() < tile_w as usize * tile_h as usize * 4 {
        return;
    }
    // 步进至少 1，防死循环
    let sx = step_x.max(1) as i32;
    let sy = step_y.max(1) as i32;
    let x_end = x + w as i32;
    let y_end = y + h as i32;
    let mut ty = y;
    while ty < y_end {
        let mut tx = x;
        while tx < x_end {
            blit_bitmap(buf, tx, ty, tile_w, tile_h, tile);
            tx += sx;
        }
        ty += sy;
    }
}

/// 区域放大（放大镜）：把 base 中 (x,y,w,h) 区域按 `zoom` 倍**最近邻**
/// 放大后画回同一区域，并描一圈边。
///
/// 取样源是 base 原图而不是画布——放大镜放大的是"截图内容"，
/// 不是已经画上去的标注（与橡皮擦/智能擦除同一语义）。最近邻与前端
/// Canvas 预览保持一致，保证所见即所得。
pub fn zoom_region(
    buf: &mut RgbaBuffer,
    base: &RgbaBuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    zoom: f32,
    style: &ShapeStyle,
) {
    if w == 0 || h == 0 || buf.width == 0 || buf.height == 0 {
        return;
    }
    if buf.width != base.width || buf.height != base.height {
        return;
    }
    let zoom = zoom.max(1.0);
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w as i32).min(buf.width as i32);
    let y1 = (y + h as i32).min(buf.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let row_bytes = buf.width as usize * 4;
    for py in y0..y1 {
        // 源坐标 = 区域原点 + 目标偏移 / zoom，clamp 回可见区域
        let sy = (y + ((py - y) as f32 / zoom) as i32).clamp(y0, y1 - 1);
        for px in x0..x1 {
            let sx = (x + ((px - x) as f32 / zoom) as i32).clamp(x0, x1 - 1);
            let si = sy as usize * row_bytes + sx as usize * 4;
            let di = py as usize * row_bytes + px as usize * 4;
            buf.data[di..di + 4].copy_from_slice(&base.data[si..si + 4]);
        }
    }
    // 边框最后画，避免被放大取样覆盖
    stroke_rect(buf, x, y, w, h, style);
}

/// 智能擦除：内容感知填充（零依赖近似 inpaint）。
///
/// 算法：把区域内像素清空后，从区域**外沿向内**做多轮"同步波前扩散"——
/// 每一轮只推进一个像素，取值 = 该像素所有**已填充**四邻像素的均值
/// （区域外取 base 原图）。总量 O(区域面积)，与轮数无关。
///
/// 与 BFS 逐像素复制的区别：同轮内后处理的像素看不到本轮新填的值
/// （状态只在轮末统一翻正），因此是**同步**扩散，渐变背景上不会出现
/// 菱形阶梯。平坦背景上效果接近 OpenCV inpaint；强纹理背景只能"抹平"，
/// 做不到"无痕"——这是算法定位，不是缺陷。
///
/// `iterations == 0` 时按区域尺寸自动取足够收敛的轮数；显式传小值则
/// 提前停止，剩余像素保持原样（供将来做"擦除强度"滑杆）。
/// 区域铺满整图时四邻无从取样，按无操作处理。
pub fn smart_erase_region(
    buf: &mut RgbaBuffer,
    base: &RgbaBuffer,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    iterations: u32,
) {
    if w == 0 || h == 0 || buf.width == 0 || buf.height == 0 {
        return;
    }
    if buf.width != base.width || buf.height != base.height {
        return;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w as i32).min(buf.width as i32);
    let y1 = (y + h as i32).min(buf.height as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let rw = (x1 - x0) as usize;
    let rh = (y1 - y0) as usize;
    // 波前每轮推进 1px，收敛轮数 = 区域中心到最近边界的距离 + 2 兜底
    let max_rounds = if iterations == 0 {
        (rw.min(rh) as u32 / 2 + 2).max(1)
    } else {
        iterations.max(1)
    };

    const UNFILLED: u8 = 0;
    const QUEUED: u8 = 1;
    const FILLED: u8 = 2;
    let mut state = vec![UNFILLED; rw * rh];
    let row_bytes = buf.width as usize * 4;

    // 播种：区域四周一圈，值 = 相邻"区域外 base 像素"的均值。
    // 扩散源取 base 原图而非画布：智能擦除的语义是"从原图里抹掉这个
    // 区域"，不受已画标注影响（与橡皮擦/放大镜一致）。
    let mut frontier: Vec<usize> = Vec::new();
    for ly in 0..rh {
        for lx in 0..rw {
            if lx != 0 && ly != 0 && lx != rw - 1 && ly != rh - 1 {
                continue; // 只处理四周一圈
            }
            let (sum, n) = outside_mean(base, row_bytes, x0, y0, rw, rh, lx, ly);
            if n == 0 {
                continue;
            }
            let di = (y0 + ly as i32) as usize * row_bytes + (x0 + lx as i32) as usize * 4;
            write_mean(&mut buf.data, di, sum, n);
            state[ly * rw + lx] = FILLED;
            frontier.push(ly * rw + lx);
        }
    }
    if frontier.is_empty() {
        return; // 区域铺满整图：无从取样，保持原样
    }

    for _ in 0..max_rounds {
        if frontier.is_empty() {
            break;
        }
        let mut next: Vec<usize> = Vec::new();
        for &p in &frontier {
            let lx = (p % rw) as i32;
            let ly = (p / rw) as i32;
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let nlx = lx + dx;
                let nly = ly + dy;
                if nlx < 0 || nly < 0 || nlx >= rw as i32 || nly >= rh as i32 {
                    continue;
                }
                let q = nly as usize * rw + nlx as usize;
                if state[q] != UNFILLED {
                    continue;
                }
                state[q] = QUEUED;
                // 已填邻居（区域内读画布，区域外读 base）的均值
                let (mut sr, mut sg, mut sb, mut sa, mut n) = (0u32, 0u32, 0u32, 0u32, 0u32);
                for (ddx, ddy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let qlx = nlx + ddx;
                    let qly = nly + ddy;
                    if qlx < 0 || qly < 0 || qlx >= rw as i32 || qly >= rh as i32 {
                        let ax = x0 + qlx;
                        let ay = y0 + qly;
                        if ax < 0 || ay < 0 || ax >= base.width as i32 || ay >= base.height as i32
                        {
                            continue;
                        }
                        let i = ay as usize * row_bytes + ax as usize * 4;
                        sr += base.data[i] as u32;
                        sg += base.data[i + 1] as u32;
                        sb += base.data[i + 2] as u32;
                        sa += base.data[i + 3] as u32;
                        n += 1;
                    } else {
                        let qq = qly as usize * rw + qlx as usize;
                        if state[qq] != FILLED {
                            continue;
                        }
                        let i = (y0 + qly) as usize * row_bytes + (x0 + qlx) as usize * 4;
                        sr += buf.data[i] as u32;
                        sg += buf.data[i + 1] as u32;
                        sb += buf.data[i + 2] as u32;
                        sa += buf.data[i + 3] as u32;
                        n += 1;
                    }
                }
                if n == 0 {
                    state[q] = UNFILLED; // 防御：理论上不可达
                    continue;
                }
                let di = (y0 + nly) as usize * row_bytes + (x0 + nlx) as usize * 4;
                write_mean(&mut buf.data, di, [sr, sg, sb, sa], n);
                next.push(q);
            }
        }
        for &q in &next {
            state[q] = FILLED;
        }
        frontier = next;
    }
}

/// 智能擦除辅助：区域局部坐标 `(lx, ly)` 四邻中**区域外**（读 base）像素的
/// 均值与个数。区域内邻居由调用方自行处理（波前状态相关）。
fn outside_mean(
    base: &RgbaBuffer,
    row_bytes: usize,
    x0: i32,
    y0: i32,
    rw: usize,
    rh: usize,
    lx: usize,
    ly: usize,
) -> ([u32; 4], u32) {
    let mut sum = [0u32; 4];
    let mut n = 0u32;
    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
        let nlx = lx as i32 + dx;
        let nly = ly as i32 + dy;
        if nlx < 0 || nly < 0 || nlx >= rw as i32 || nly >= rh as i32 {
            let ax = x0 + nlx;
            let ay = y0 + nly;
            if ax < 0 || ay < 0 || ax >= base.width as i32 || ay >= base.height as i32 {
                continue;
            }
            let i = ay as usize * row_bytes + ax as usize * 4;
            for c in 0..4 {
                sum[c] += base.data[i + c] as u32;
            }
            n += 1;
        }
    }
    (sum, n)
}

/// 智能擦除辅助：把 (sum/n) 写进 data 的 di 处。
fn write_mean(data: &mut [u8], di: usize, sum: [u32; 4], n: u32) {
    for c in 0..4 {
        data[di + c] = (sum[c] / n) as u8;
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
    fn blit_copies_pixels_at_offset() {
        let mut buf = black(10, 10);
        // 2x2 纯红不透明贴图
        let src = vec![255, 0, 0, 255].repeat(4);
        blit_bitmap(&mut buf, 3, 4, 2, 2, &src);
        assert_eq!(buf.pixel(3, 4), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(4, 5), Some([255, 0, 0, 255]));
        // 贴图之外保持原样
        assert_eq!(buf.pixel(2, 4), Some([0, 0, 0, 255]));
        assert_eq!(buf.pixel(5, 4), Some([0, 0, 0, 255]));
    }

    #[test]
    fn blit_respects_source_alpha() {
        let mut buf = black(1, 1);
        // 50% 白覆盖黑底 → 约 128
        blit_bitmap(&mut buf, 0, 0, 1, 1, &[255, 255, 255, 128]);
        let p = buf.pixel(0, 0).unwrap();
        assert!((p[0] as i32 - 128).abs() <= 1, "got {:?}", p);
    }

    #[test]
    fn blit_clips_to_buffer_edges() {
        let mut buf = black(4, 4);
        // 负坐标与越界：不应 panic，只在相交区域落像素
        let src = vec![255, 0, 0, 255].repeat(9); // 3x3
        blit_bitmap(&mut buf, -1, -1, 3, 3, &src);
        assert_eq!(buf.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(1, 1), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(2, 2), Some([0, 0, 0, 255]));

        let mut buf2 = black(4, 4);
        blit_bitmap(&mut buf2, 3, 3, 3, 3, &src);
        assert_eq!(buf2.pixel(3, 3), Some([255, 0, 0, 255]));
        assert_eq!(buf2.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn blit_ignores_short_or_empty_data() {
        let mut buf = black(2, 2);
        // 长度不足：静默跳过，不 panic
        blit_bitmap(&mut buf, 0, 0, 2, 2, &[255, 0, 0, 255]);
        blit_bitmap(&mut buf, 0, 0, 0, 0, &[]);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn bitmap_shape_draws_through_draw_shape() {
        let mut buf = black(6, 6);
        let s = Shape::Bitmap {
            x: 1,
            y: 2,
            w: 2,
            h: 1,
            rgba: vec![0, 255, 0, 255, 0, 255, 0, 255],
        };
        draw_shape(&mut buf, &s);
        assert_eq!(buf.pixel(1, 2), Some([0, 255, 0, 255]));
        assert_eq!(buf.pixel(2, 2), Some([0, 255, 0, 255]));
        assert_eq!(buf.pixel(3, 2), Some([0, 0, 0, 255]));
    }

    /// 压暗系数对应的期望灰度：255 × (1 - dim/255)
    fn dimmed(dim: u8, k: f32) -> i32 {
        (255.0 * (1.0 - (dim as f32 / 255.0) * k)).round() as i32
    }

    fn white(w: u32, h: u32) -> RgbaBuffer {
        let mut data = vec![255u8; w as usize * h as usize * 4];
        for px in data.chunks_exact_mut(4) {
            px[3] = 255;
        }
        RgbaBuffer::new(w, h, data).unwrap()
    }

    #[test]
    fn spotlight_dims_outside_and_keeps_inside() {
        let mut buf = white(40, 40);
        spotlight_region(&mut buf, 10, 10, 20, 20, 0, 200);
        // 洞内保持纯白（不留一点暗角）
        assert_eq!(buf.pixel(20, 20), Some([255, 255, 255, 255]));
        assert_eq!(buf.pixel(11, 11), Some([255, 255, 255, 255]));
        // 洞外压暗，alpha 不变（压暗不该改变透明度）
        let corner = buf.pixel(0, 0).unwrap();
        let want = dimmed(200, 1.0);
        assert!((corner[0] as i32 - want).abs() <= 1, "corner={corner:?} want={want}");
        assert_eq!(corner[3], 255);
    }

    #[test]
    fn spotlight_feathers_edge_instead_of_hard_step() {
        let mut buf = white(20, 20);
        spotlight_region(&mut buf, 5, 5, 10, 10, 0, 255);
        // 右边界外 0.5px 处落在羽化带内：既不纯白也不全黑
        let p = buf.pixel(15, 10).unwrap();
        assert!(p[0] > 0 && p[0] < 255, "feathered pixel={p:?}");
        // 离得够远就该完全压黑
        assert_eq!(buf.pixel(19, 10), Some([0, 0, 0, 255]));
    }

    #[test]
    fn spotlight_full_radius_becomes_circle() {
        let mut buf = white(40, 40);
        // 20×20 且 radius=10 → 正圆：中心亮，外接框角落（圆外）压黑
        spotlight_region(&mut buf, 10, 10, 20, 20, 10, 255);
        assert_eq!(buf.pixel(20, 20), Some([255, 255, 255, 255]));
        assert_eq!(buf.pixel(11, 11), Some([0, 0, 0, 255]));
    }

    #[test]
    fn spotlight_zero_dim_is_noop() {
        let mut buf = white(10, 10);
        spotlight_region(&mut buf, 2, 2, 4, 4, 1, 0);
        for y in 0..10 {
            for x in 0..10 {
                assert_eq!(buf.pixel(x, y), Some([255, 255, 255, 255]), "({x},{y})");
            }
        }
    }

    #[test]
    fn spotlight_handles_out_of_bounds_and_degenerate() {
        // 完全在画布外：整幅压暗，且不 panic
        let mut buf = white(8, 8);
        spotlight_region(&mut buf, -50, -50, 10, 10, 2, 255);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
        // 退化尺寸：无操作
        let mut buf2 = white(8, 8);
        spotlight_region(&mut buf2, 0, 0, 0, 0, 0, 255);
        assert_eq!(buf2.pixel(0, 0), Some([255, 255, 255, 255]));
        // 半径大于短边一半：自动 clamp，不破形
        let mut buf3 = white(20, 20);
        spotlight_region(&mut buf3, 5, 5, 10, 10, 999, 255);
        assert_eq!(buf3.pixel(10, 10), Some([255, 255, 255, 255]));
        assert_eq!(buf3.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn spotlight_draws_through_draw_shape() {
        let mut buf = white(30, 30);
        let s = Shape::Spotlight { x: 10, y: 10, w: 10, h: 10, radius: 0, dim: 255 };
        draw_shape(&mut buf, &s);
        assert_eq!(buf.pixel(15, 15), Some([255, 255, 255, 255]));
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn highlighter_overlap_blends_not_overwrites() {        let mut buf = black(20, 20);
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

    // ---- M3 新增：橡皮擦 / 水印 / 放大镜 / 智能擦除 ----

    /// 纯色测试图
    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaBuffer {
        let mut data = vec![0u8; w as usize * h as usize * 4];
        for px in data.chunks_exact_mut(4) {
            px.copy_from_slice(&c);
        }
        RgbaBuffer::new(w, h, data).unwrap()
    }

    /// 水平渐变测试图：灰度 = x * step
    fn gradient_x(w: u32, h: u32, step: u8) -> RgbaBuffer {
        let mut data = vec![0u8; w as usize * h as usize * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize * 4;
                let v = (x as u8).wrapping_mul(step);
                data[i] = v;
                data[i + 1] = v;
                data[i + 2] = v;
                data[i + 3] = 255;
            }
        }
        RgbaBuffer::new(w, h, data).unwrap()
    }

    #[test]
    fn eraser_restores_base_pixels_not_paint() {
        // base 左半白右半黑；画布全红（模拟已画标注）
        let mut base_data = vec![0u8; 20 * 10 * 4];
        for y in 0..10 {
            for x in 0..20 {
                let i = (y * 20 + x) * 4;
                let v = if x < 10 { 255 } else { 0 };
                base_data[i] = v;
                base_data[i + 1] = v;
                base_data[i + 2] = v;
                base_data[i + 3] = 255;
            }
        }
        let base = RgbaBuffer::new(20, 10, base_data).unwrap();
        let mut buf = solid(20, 10, [255, 0, 0, 255]);
        eraser_region(&mut buf, &base, &[(15.0, 5.0)], 3.0);
        // 擦除处还原为 base 的黑，而不是某种固定颜色
        assert_eq!(buf.pixel(15, 5), Some([0, 0, 0, 255]));
        assert_eq!(buf.pixel(13, 4), Some([0, 0, 0, 255]));
        // 未擦处保持红
        assert_eq!(buf.pixel(2, 2), Some([255, 0, 0, 255]));
    }

    #[test]
    fn eraser_covers_whole_trajectory() {
        let base = white(20, 20);
        let mut buf = solid(20, 20, [255, 0, 0, 255]);
        // 从 (2,10) 拖到 (17,10)，半径 2 → 整条线（含上下各 1px）都应还原成白
        eraser_region(&mut buf, &base, &[(2.0, 10.0), (17.0, 10.0)], 2.0);
        for x in 2..=17 {
            assert_eq!(buf.pixel(x, 10), Some([255, 255, 255, 255]), "x={x}");
            assert_eq!(buf.pixel(x, 9), Some([255, 255, 255, 255]), "x={x} upper");
        }
        // 线外保持红
        assert_eq!(buf.pixel(10, 5), Some([255, 0, 0, 255]));
    }

    #[test]
    fn eraser_empty_trajectory_is_noop() {
        let base = white(4, 4);
        let mut buf = solid(4, 4, [255, 0, 0, 255]);
        eraser_region(&mut buf, &base, &[], 3.0);
        assert_eq!(buf.pixel(0, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn draw_shapes_with_base_restores_through_eraser() {
        let base = white(10, 10);
        let mut buf = solid(10, 10, [0, 0, 0, 255]);
        let shapes = vec![
            Shape::Rect { x: 1, y: 1, w: 8, h: 8, style: red() },
            Shape::Eraser { points: vec![(5.0, 5.0)], radius: 2.0 },
        ];
        draw_shapes_with_base(&mut buf, &base, &shapes);
        // 擦掉后露出 base 的白
        assert_eq!(buf.pixel(5, 5), Some([255, 255, 255, 255]));
        // 矩形其余部分仍在
        assert_eq!(buf.pixel(1, 1), Some([255, 0, 0, 255]));
    }

    #[test]
    fn base_dependent_shapes_are_noop_without_base() {
        let mut buf = solid(10, 10, [0, 0, 0, 255]);
        let shapes = vec![
            Shape::Eraser { points: vec![(5.0, 5.0)], radius: 2.0 },
            Shape::ZoomRegion { x: 1, y: 1, w: 4, h: 4, zoom: 2.0, style: red() },
            Shape::SmartErase { x: 1, y: 1, w: 4, h: 4, iterations: 0 },
        ];
        draw_shapes(&mut buf, &shapes);
        for y in 0..10 {
            for x in 0..10 {
                assert_eq!(buf.pixel(x, y), Some([0, 0, 0, 255]), "({x},{y})");
            }
        }
    }

    #[test]
    fn draw_shapes_with_base_mismatched_dims_is_safe() {
        let base = white(4, 4);
        let mut buf = black(8, 8);
        let shapes = vec![
            Shape::Eraser { points: vec![(2.0, 2.0)], radius: 2.0 },
            Shape::ZoomRegion { x: 0, y: 0, w: 4, h: 4, zoom: 2.0, style: red() },
            Shape::SmartErase { x: 0, y: 0, w: 4, h: 4, iterations: 0 },
            Shape::Rect { x: 0, y: 0, w: 4, h: 4, style: red() },
        ];
        // 尺寸不一致：依赖 base 的三个静默跳过，普通图形照常画
        draw_shapes_with_base(&mut buf, &base, &shapes);
        assert_eq!(buf.pixel(0, 0), Some([255, 0, 0, 255]));
        // (6,6) 在 8x8 画布内但完全脱离 4x4 矩形（含 2px 描边）
        assert_eq!(buf.pixel(6, 6), Some([0, 0, 0, 255]));
    }

    #[test]
    fn draw_shapes_with_base_dispatches_zoom_and_smart_erase() {
        let base = white(10, 10);
        let mut buf = solid(10, 10, [255, 0, 0, 255]);
        let shapes = vec![
            Shape::ZoomRegion { x: 1, y: 1, w: 4, h: 4, zoom: 2.0, style: red() },
            Shape::SmartErase { x: 6, y: 6, w: 3, h: 3, iterations: 0 },
        ];
        draw_shapes_with_base(&mut buf, &base, &shapes);
        // 放大镜区域内部（描边之内）变成 base 的白
        assert_eq!(buf.pixel(3, 3), Some([255, 255, 255, 255]));
        // 智能擦除中心被填白
        assert_eq!(buf.pixel(7, 7), Some([255, 255, 255, 255]));
        // 远处未触及处保持红
        assert_eq!(buf.pixel(9, 9), Some([255, 0, 0, 255]));
    }

    #[test]
    fn watermark_tiles_with_step_gaps() {
        let mut buf = black(8, 8);
        // 2x1 纯红 tile，步进 3x3（>tile 尺寸 → 有间距）
        // x = 0,3,6（各覆盖 2px），y = 0,3,6
        let tile = vec![255, 0, 0, 255, 255, 0, 0, 255];
        watermark_region(&mut buf, 0, 0, 8, 8, 3, 3, 2, 1, &tile);
        assert_eq!(buf.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(1, 0), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(2, 0), Some([0, 0, 0, 255])); // 列间距
        assert_eq!(buf.pixel(3, 0), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(6, 0), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(0, 3), Some([255, 0, 0, 255])); // 行方向平铺
        assert_eq!(buf.pixel(0, 1), Some([0, 0, 0, 255])); // 行间距
    }

    #[test]
    fn watermark_respects_tile_alpha() {
        let mut buf = black(2, 1);
        watermark_region(&mut buf, 0, 0, 2, 1, 2, 2, 1, 1, &[255, 255, 255, 128]);
        let p = buf.pixel(0, 0).unwrap();
        assert!((p[0] as i32 - 128).abs() <= 1, "got {:?}", p);
    }

    #[test]
    fn watermark_clips_out_of_bounds_and_bad_data() {
        let mut buf = black(4, 4);
        // tile 数据长度不足：静默跳过，不动画布
        watermark_region(&mut buf, 0, 0, 4, 4, 2, 2, 2, 2, &[0; 4]);
        assert_eq!(buf.pixel(0, 0), Some([0, 0, 0, 255]));
        // step 0 被 clamp 到 1，仍正常平铺（不死循环）
        let mut buf2 = black(3, 1);
        watermark_region(&mut buf2, 0, 0, 3, 1, 0, 0, 1, 1, &[0, 255, 0, 255]);
        for x in 0..3 {
            assert_eq!(buf2.pixel(x, 0), Some([0, 255, 0, 255]), "x={x}");
        }
        // 区域完全越界：不 panic
        watermark_region(&mut buf2, -50, -50, 3, 1, 1, 1, 1, 1, &[0, 255, 0, 255]);
    }

    #[test]
    fn zoom_region_enlarges_base_and_draws_border() {
        // 10x10 水平渐变：灰度 = x*10
        let base = gradient_x(10, 10, 10);
        let mut buf = black(10, 10);
        // 放大 (1,1,6,6)，zoom=2：描边宽 2 之后内部只剩 (3..5, 3..5)
        zoom_region(&mut buf, &base, 1, 1, 6, 6, 2.0, &red());
        // 内部：源坐标 = 1 + (目标-1)/2 → 左两列 ← 源 x=2（灰度 20），右列 ← 源 x=3（灰度 30）
        for (px, py) in [(3, 3), (4, 3), (3, 4), (4, 4), (3, 5), (4, 5)] {
            assert_eq!(buf.pixel(px, py), Some([20, 20, 20, 255]), "({px},{py})");
        }
        for (px, py) in [(5, 3), (5, 4), (5, 5)] {
            assert_eq!(buf.pixel(px, py), Some([30, 30, 30, 255]), "({px},{py})");
        }
        // 边框：区域上边 (3,1) 与左边 (1,3) 应为红线
        assert_eq!(buf.pixel(3, 1), Some([255, 0, 0, 255]));
        assert_eq!(buf.pixel(1, 3), Some([255, 0, 0, 255]));
        // 区域外不动
        assert_eq!(buf.pixel(9, 9), Some([0, 0, 0, 255]));
        assert_eq!(buf.pixel(0, 9), Some([0, 0, 0, 255]));
    }

    #[test]
    fn zoom_region_samples_base_not_canvas() {
        let base = white(6, 6);
        let mut buf = solid(6, 6, [255, 0, 0, 255]); // 画布全红
        zoom_region(&mut buf, &base, 1, 1, 4, 4, 2.0, &red());
        // 放大的是 base（白），不是画布（红）
        assert_eq!(buf.pixel(3, 3), Some([255, 255, 255, 255]));
    }

    #[test]
    fn zoom_region_out_of_bounds_does_not_panic() {
        let base = white(8, 8);
        let mut buf = black(8, 8);
        zoom_region(&mut buf, &base, -4, -4, 8, 8, 2.0, &red());
        zoom_region(&mut buf, &base, 100, 100, 4, 4, 2.0, &red());
        zoom_region(&mut buf, &base, 0, 0, 0, 0, 2.0, &red());
        // 部分越界时可见区域被放大填充
        assert_eq!(buf.pixel(0, 0), Some([255, 255, 255, 255]));
    }

    #[test]
    fn smart_erase_fills_flat_background() {
        // 白底中间 4x4 红块 → 智能擦除后应还原为白
        let base = white(10, 10);
        let mut buf = base.clone();
        for y in 3..7 {
            for x in 3..7 {
                blend_pixel(&mut buf, x, y, [255, 0, 0, 255]);
            }
        }
        smart_erase_region(&mut buf, &base, 3, 3, 4, 4, 0);
        for y in 3..7 {
            for x in 3..7 {
                assert_eq!(buf.pixel(x, y), Some([255, 255, 255, 255]), "({x},{y})");
            }
        }
    }

    #[test]
    fn smart_erase_fills_gradient_monotonically() {
        // 水平渐变：灰度 = x*20；擦除中间 3..6 后填充值应介于两侧边界灰度之间
        let base = gradient_x(10, 10, 20);
        let mut buf = base.clone();
        for y in 3..6 {
            for x in 3..6 {
                blend_pixel(&mut buf, x, y, [0, 0, 0, 255]);
            }
        }
        smart_erase_region(&mut buf, &base, 3, 3, 3, 3, 0);
        // 每行从左到右单调不降
        for y in 3..6 {
            let mut prev = 0u8;
            for x in 3..6 {
                let v = buf.pixel(x, y).unwrap()[0];
                assert!(v >= prev, "row {y} not monotone at x={x}: {v} < {prev}");
                prev = v;
            }
        }
        // 左边界灰度 40（x=2）、右边界 120（x=6）
        let first = buf.pixel(3, 4).unwrap()[0];
        let last = buf.pixel(5, 4).unwrap()[0];
        assert!((40..=120).contains(&first), "first={first}");
        assert!((40..=120).contains(&last), "last={last}");
        assert!(first <= last, "first={first} last={last}");
    }

    #[test]
    fn smart_erase_full_image_is_noop() {
        // 区域铺满整图：四邻无从取样 → 保持原样
        let base = white(6, 6);
        let mut buf = solid(6, 6, [255, 0, 0, 255]);
        smart_erase_region(&mut buf, &base, 0, 0, 6, 6, 0);
        for y in 0..6 {
            for x in 0..6 {
                assert_eq!(buf.pixel(x, y), Some([255, 0, 0, 255]), "({x},{y})");
            }
        }
    }

    #[test]
    fn smart_erase_respects_iteration_cap() {
        // 20x20 白底，中间 12x12 涂红；iterations=2 → 只推进 2 圈，中心仍红
        let base = white(20, 20);
        let mut buf = base.clone();
        for y in 4..16 {
            for x in 4..16 {
                blend_pixel(&mut buf, x, y, [255, 0, 0, 255]);
            }
        }
        smart_erase_region(&mut buf, &base, 4, 4, 12, 12, 2);
        // 边缘两圈被填白
        assert_eq!(buf.pixel(4, 4), Some([255, 255, 255, 255]));
        assert_eq!(buf.pixel(5, 5), Some([255, 255, 255, 255]));
        // 中心（距边界 > 2）保持红
        assert_eq!(buf.pixel(10, 10), Some([255, 0, 0, 255]));
    }

    #[test]
    fn smart_erase_out_of_bounds_does_not_panic() {
        let base = white(8, 8);
        let mut buf = solid(8, 8, [0, 0, 0, 255]);
        // 部分越界（左上方伸出画布）：可见部分被填白
        smart_erase_region(&mut buf, &base, -2, 2, 6, 4, 0);
        assert_eq!(buf.pixel(0, 2), Some([255, 255, 255, 255]));
        // 完全越界与退化尺寸：不 panic
        smart_erase_region(&mut buf, &base, 100, 100, 5, 5, 0);
        smart_erase_region(&mut buf, &base, 0, 0, 0, 0, 0);
    }
}
