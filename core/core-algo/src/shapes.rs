//! 标注图形数据模型（M3 标注工具）。
//!
//! 设计约束（同本 crate 顶层）：
//! - 零依赖、纯值类型、可 `Copy`/`Clone`/`PartialEq`，C++ 移植是机械翻译；
//! - 坐标为**物理像素**，原点为**裁剪图左上角**（前端在发送前已把
//!   显示器原点与选区原点减掉，并乘以 DPI 缩放因子）；
//! - 颜色统一 RGBA8（`[r, g, b, a]`），荧光笔/半透明效果通过 alpha 表达；
//! - 渲染与数据分离：本模块只描述"画什么"，`draw` 模块负责"怎么画"，
//!   前端 Canvas 预览与 Rust 光栅化输出共用同一份数据，保证所见即所得。

use crate::Rect;

/// 线条样式：颜色 + 线宽（物理像素）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeStyle {
    /// RGBA8
    pub color: [u8; 4],
    /// 线宽（物理像素），最小 1.0
    pub width: f32,
}

impl ShapeStyle {
    pub fn new(color: [u8; 4], width: f32) -> Self {
        Self {
            color,
            width: if width < 1.0 { 1.0 } else { width },
        }
    }
}

impl Default for ShapeStyle {
    fn default() -> Self {
        // 默认：不透明红，2px（截图标注的经典配色）
        Self::new([230, 45, 45, 255], 2.0)
    }
}

/// 一个标注图形。
///
/// 坐标约定见模块文档。`Pen`/`Highlighter` 的点序列为按下→移动→抬起的
/// 原始轨迹（前端已做长度去重）。
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// 矩形描边
    Rect {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: ShapeStyle,
    },
    /// 椭圆描边（外接矩形）
    Ellipse {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        style: ShapeStyle,
    },
    /// 箭头（直线 + 两翼）
    Arrow {
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
        style: ShapeStyle,
    },
    /// 铅笔（不透明细线，圆头笔帽）
    Pen {
        points: Vec<(f32, f32)>,
        style: ShapeStyle,
    },
    /// 荧光笔（半透明粗线，圆头笔帽）
    Highlighter {
        points: Vec<(f32, f32)>,
        style: ShapeStyle,
    },
    /// 马赛克（块平均像素化）
    Mosaic {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        /// 块边长（物理像素），最小 2
        block: u32,
    },
    /// 位图图层：由 UI 层预先光栅化好的 RGBA8 贴图，核心层只做 alpha 合成。
    ///
    /// 为什么文字类标注要走位图而不是在这里画字：渲染文字需要字体文件、
    /// CJK 回退与输入法，属 UI 层能力。若把字体库引进 core-algo，会破坏
    /// 本 crate「零依赖 + 可机械翻译 C++」的硬约束（鸿蒙 PC 路线），
    /// 中文字体（≥10MB）也不该进安装包。故文字由前端 Canvas 用系统字体
    /// 光栅化成位图，核心层只负责合成——既守住约束，又天然支持
    /// 中文/emoji/任意系统字体，并且所见即所得由构造保证。
    Bitmap {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        /// RGBA8，行优先，长度 = w*h*4，**非预乘 alpha**
        rgba: Vec<u8>,
    },
    /// 聚光灯：把**圆角矩形区域之外**的像素压暗，区域本身保持原亮度。
    ///
    /// 与其他图形不同，它不改写任何几何形状，只是对底图做一次
    /// "区域外乘性压暗"，因此可以与底图逐像素合成而不需要额外的图层。
    /// 用的是圆角矩形有符号距离场（SDF），边缘再按 1.5px 羽化，
    /// 避免圆角处出现硬锯齿。
    Spotlight {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        /// 圆角半径（物理像素），实际生效值会被 clamp 到 min(w,h)/2
        radius: u32,
        /// 压暗强度：0 = 不压暗，255 = 全黑
        dim: u8,
    },
    /// 橡皮擦：把轨迹区域**还原成底图**（擦掉已画上去的标注）。
    ///
    /// 与"用白色涂抹"有本质区别：橡皮擦不引入任何颜色，它从 `base`
    /// （绘制任何标注之前的原始裁剪图）把像素拷回来。因此调用方必须
    /// 通过 `draw::draw_shapes_with_base` 传入 base 缓冲；单独调用
    /// `draw_shape` 时本变体按"无操作"处理（没有 base 就无从还原）。
    Eraser {
        /// 轨迹点（物理像素）
        points: Vec<(f32, f32)>,
        /// 擦除半径（物理像素）
        radius: f32,
    },
    /// 水印：把一小块位图按步进平铺进指定区域（半透明文字/图片水印）。
    ///
    /// 位图由 UI 层用系统字体光栅化后上传（见 `Bitmap` 的说明），
    /// 核心层只做重复 blit——字体、CJK、emoji 都不进核心层。
    Watermark {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        /// 平铺步进（物理像素）：>tile 的部分即水印间距
        step_x: u32,
        step_y: u32,
        /// 单块水印位图尺寸（物理像素）
        tile_w: u32,
        tile_h: u32,
        /// RGBA8，行优先，长度 = tile_w*tile_h*4，**非预乘 alpha**
        tile: Vec<u8>,
    },
    /// 区域放大（放大镜）：把 base 中 (x,y,w,h) 区域按 `zoom` 倍放大后
    /// 画回同一区域，外加一圈描边。取样用最近邻，与前端预览一致。
    ///
    /// 同样依赖 base：放大的是"原图内容"，不是已画上的标注。
    ZoomRegion {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        /// 放大倍数（≥1.0，可为小数）
        zoom: f32,
        /// 描边颜色（放大区域边框）
        style: ShapeStyle,
    },
    /// 智能擦除：内容感知填充（零依赖近似 inpaint）。
    ///
    /// 做法：把区域内像素清空，再从区域**外沿向内**做多轮"最近外界
    /// 像素扩散"（每轮只允许推进一个像素，取该像素四个已填充邻居的
    /// 均值），收敛后得到与周围纹理连续的填充。对小物体（文字、图标、
    /// 人脸、水印）在平坦背景上效果接近 OpenCV inpaint；强纹理背景
    /// 只能做到"抹平"，做不到"无痕"——这是算法定位，不是缺陷。
    SmartErase {
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        /// 扩散轮数上限（0 = 自动，按区域短边估算）
        iterations: u32,
    },
}

impl Shape {
    /// 平移（供整幅标注整体偏移用，如选区校正）。
    pub fn translate(&mut self, dx: f32, dy: f32) {
        match self {
            Shape::Rect { x, y, .. } | Shape::Ellipse { x, y, .. } => {
                *x += dx as i32;
                *y += dy as i32;
            }
            Shape::Arrow { x1, y1, x2, y2, .. } => {
                *x1 += dx as i32;
                *y1 += dy as i32;
                *x2 += dx as i32;
                *y2 += dy as i32;
            }
            Shape::Pen { points, .. } | Shape::Highlighter { points, .. } => {
                for p in points.iter_mut() {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
            Shape::Mosaic { x, y, .. } | Shape::Bitmap { x, y, .. } => {
                *x += dx as i32;
                *y += dy as i32;
            }
            Shape::Eraser { points, .. } => {
                // 橡皮擦平移整条轨迹（没有点时不移动）
                for p in points.iter_mut() {
                    p.0 += dx;
                    p.1 += dy;
                }
            }
            Shape::Spotlight { x, y, .. }
            | Shape::Watermark { x, y, .. }
            | Shape::ZoomRegion { x, y, .. }
            | Shape::SmartErase { x, y, .. } => {
                *x += dx as i32;
                *y += dy as i32;
            }
        }
    }

    /// 近似外接矩形（用于"本次标注是否为空"等粗判断，不保证紧致）。
    pub fn bounds(&self) -> Option<Rect> {
        match self {
            Shape::Rect { x, y, w, h, .. }
            | Shape::Ellipse { x, y, w, h, .. }
            | Shape::Mosaic { x, y, w, h, .. }
            | Shape::Spotlight { x, y, w, h, .. }
            | Shape::Bitmap { x, y, w, h, .. }
            | Shape::Watermark { x, y, w, h, .. }
            | Shape::ZoomRegion { x, y, w, h, .. }
            | Shape::SmartErase { x, y, w, h, .. } => Some(Rect {
                x: *x,
                y: *y,
                w: *w,
                h: *h,
            }),
            Shape::Arrow { x1, y1, x2, y2, .. } => Some(Rect::from_drag(*x1, *y1, *x2, *y2)),
            Shape::Eraser { points, radius } => {
                if points.is_empty() {
                    return None;
                }
                let r = *radius;
                let mut min_x = f32::MAX;
                let mut min_y = f32::MAX;
                let mut max_x = f32::MIN;
                let mut max_y = f32::MIN;
                for &(px, py) in points {
                    min_x = min_x.min(px - r);
                    min_y = min_y.min(py - r);
                    max_x = max_x.max(px + r);
                    max_y = max_y.max(py + r);
                }
                Some(Rect {
                    x: min_x as i32,
                    y: min_y as i32,
                    w: (max_x - min_x).max(0.0) as u32,
                    h: (max_y - min_y).max(0.0) as u32,
                })
            }
            Shape::Pen { points, .. } | Shape::Highlighter { points, .. } => {
                if points.is_empty() {
                    return None;
                }
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
                Some(Rect {
                    x: min_x as i32,
                    y: min_y as i32,
                    w: (max_x - min_x).max(0.0) as u32,
                    h: (max_y - min_y).max(0.0) as u32,
                })
            }
        }
    }

    /// 轨迹类图形（Pen/Highlighter/Eraser）的点序列，其他返回 None。
    pub fn points(&self) -> Option<&[(f32, f32)]> {
        match self {
            Shape::Pen { points, .. }
            | Shape::Highlighter { points, .. }
            | Shape::Eraser { points, .. } => Some(points),
            _ => None,
        }
    }

    /// 线宽（马赛克/智能擦除无线宽，返回块边长/0 近似；位图图层与水印
    /// 平铺和线宽无关，返回 0；聚光灯返回圆角半径近似；橡皮擦返回半径）。
    pub fn width_or_block(&self) -> f32 {
        match self {
            Shape::Rect { style, .. }
            | Shape::Ellipse { style, .. }
            | Shape::Arrow { style, .. }
            | Shape::Pen { style, .. }
            | Shape::Highlighter { style, .. } => style.width,
            Shape::Mosaic { block, .. } => *block as f32,
            Shape::Bitmap { .. } => 0.0,
            Shape::Spotlight { radius, .. } => *radius as f32,
            Shape::Eraser { radius, .. } => *radius,
            Shape::Watermark { .. } => 0.0,
            Shape::ZoomRegion { style, .. } => style.width,
            Shape::SmartErase { .. } => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> ShapeStyle {
        ShapeStyle::new([1, 2, 3, 255], 2.0)
    }

    #[test]
    fn style_clamps_min_width() {
        assert_eq!(ShapeStyle::new([0, 0, 0, 255], 0.2).width, 1.0);
        assert_eq!(style().width, 2.0);
    }

    #[test]
    fn default_style_is_red() {
        assert_eq!(ShapeStyle::default().color, [230, 45, 45, 255]);
    }

    #[test]
    fn translate_moves_all_variants() {
        let mut r = Shape::Rect { x: 1, y: 2, w: 3, h: 4, style: style() };
        r.translate(10.0, -5.0);
        assert_eq!(r.bounds(), Some(Rect { x: 11, y: -3, w: 3, h: 4 }));

        let mut a = Shape::Arrow { x1: 0, y1: 0, x2: 5, y2: 5, style: style() };
        a.translate(1.0, 1.0);
        assert_eq!(a.bounds(), Some(Rect { x: 1, y: 1, w: 5, h: 5 }));

        let mut p = Shape::Pen {
            points: vec![(0.0, 0.0), (2.0, 2.0)],
            style: style(),
        };
        p.translate(-1.0, -1.0);
        assert_eq!(p.points(), Some(&[(-1.0, -1.0), (1.0, 1.0)][..]));

        let mut m = Shape::Mosaic { x: 0, y: 0, w: 10, h: 10, block: 8 };
        m.translate(5.0, 5.0);
        assert_eq!(m.bounds(), Some(Rect { x: 5, y: 5, w: 10, h: 10 }));
    }

    #[test]
    fn bounds_of_pen_tracks_extremes() {
        let p = Shape::Pen {
            points: vec![(3.0, 4.0), (10.0, 1.0), (6.0, 8.0)],
            style: style(),
        };
        assert_eq!(p.bounds(), Some(Rect { x: 3, y: 1, w: 7, h: 7 }));
    }

    #[test]
    fn empty_pen_has_no_bounds() {
        let p = Shape::Pen { points: vec![], style: style() };
        assert_eq!(p.bounds(), None);
        assert_eq!(p.points(), Some(&[][..]));
    }

    #[test]
    fn non_trajectory_has_no_points() {
        let r = Shape::Rect { x: 0, y: 0, w: 1, h: 1, style: style() };
        assert_eq!(r.points(), None);
        assert_eq!(r.width_or_block(), 2.0);
    }

    #[test]
    fn bitmap_translates_and_reports_bounds() {
        let mut b = Shape::Bitmap {
            x: 10,
            y: 20,
            w: 30,
            h: 8,
            rgba: vec![0; 30 * 8 * 4],
        };
        assert_eq!(b.bounds(), Some(Rect { x: 10, y: 20, w: 30, h: 8 }));
        b.translate(-4.0, 6.0);
        assert_eq!(b.bounds(), Some(Rect { x: 6, y: 26, w: 30, h: 8 }));
        // 位图与线宽无关
        assert_eq!(b.width_or_block(), 0.0);
        assert_eq!(b.points(), None);
    }

    #[test]
    fn spotlight_translates_and_reports_bounds() {
        let mut s = Shape::Spotlight { x: 5, y: 6, w: 40, h: 20, radius: 8, dim: 150 };
        assert_eq!(s.bounds(), Some(Rect { x: 5, y: 6, w: 40, h: 20 }));
        assert_eq!(s.width_or_block(), 8.0);
        assert_eq!(s.points(), None);
        s.translate(2.0, -3.0);
        assert_eq!(s.bounds(), Some(Rect { x: 7, y: 3, w: 40, h: 20 }));
    }

    #[test]
    fn eraser_translates_trajectory_and_reports_bounds() {
        let mut e = Shape::Eraser {
            points: vec![(10.0, 10.0), (20.0, 15.0)],
            radius: 4.0,
        };
        // 半径参与 bounds
        assert_eq!(e.bounds(), Some(Rect { x: 6, y: 6, w: 18, h: 13 }));
        // 橡皮擦是轨迹类图形，points() 应可读
        assert_eq!(e.points(), Some(&[(10.0, 10.0), (20.0, 15.0)][..]));
        // 线宽查询返回半径
        assert_eq!(e.width_or_block(), 4.0);
        e.translate(5.0, -5.0);
        assert_eq!(e.points(), Some(&[(15.0, 5.0), (25.0, 10.0)][..]));
        assert_eq!(e.bounds(), Some(Rect { x: 11, y: 1, w: 18, h: 13 }));
    }

    #[test]
    fn empty_eraser_has_no_bounds() {
        let e = Shape::Eraser { points: vec![], radius: 3.0 };
        assert_eq!(e.bounds(), None);
        assert_eq!(e.points(), Some(&[][..]));
    }

    #[test]
    fn watermark_reports_rect_bounds() {
        let wm = Shape::Watermark {
            x: 4,
            y: 8,
            w: 100,
            h: 60,
            step_x: 50,
            step_y: 30,
            tile_w: 40,
            tile_h: 16,
            tile: vec![0; 40 * 16 * 4],
        };
        assert_eq!(wm.bounds(), Some(Rect { x: 4, y: 8, w: 100, h: 60 }));
        assert_eq!(wm.width_or_block(), 0.0);
        assert_eq!(wm.points(), None);
    }

    #[test]
    fn zoom_region_reports_style_width() {
        let z = Shape::ZoomRegion {
            x: 0,
            y: 0,
            w: 80,
            h: 80,
            zoom: 2.5,
            style: ShapeStyle::new([0, 0, 0, 255], 3.0),
        };
        assert_eq!(z.bounds(), Some(Rect { x: 0, y: 0, w: 80, h: 80 }));
        assert_eq!(z.width_or_block(), 3.0);
    }

    #[test]
    fn smart_erase_reports_rect_bounds() {
        let s = Shape::SmartErase { x: 2, y: 3, w: 30, h: 20, iterations: 0 };
        assert_eq!(s.bounds(), Some(Rect { x: 2, y: 3, w: 30, h: 20 }));
        assert_eq!(s.width_or_block(), 0.0);
        assert_eq!(s.points(), None);
    }

    #[test]
    fn translate_moves_new_rect_like_variants() {
        let mut wm = Shape::Watermark {
            x: 0, y: 0, w: 10, h: 10,
            step_x: 10, step_y: 10, tile_w: 5, tile_h: 5,
            tile: vec![0; 5 * 5 * 4],
        };
        wm.translate(3.0, 4.0);
        assert_eq!(wm.bounds(), Some(Rect { x: 3, y: 4, w: 10, h: 10 }));

        let mut z = Shape::ZoomRegion {
            x: 0, y: 0, w: 10, h: 10, zoom: 2.0, style: style(),
        };
        z.translate(-1.0, -1.0);
        assert_eq!(z.bounds(), Some(Rect { x: -1, y: -1, w: 10, h: 10 }));

        let mut se = Shape::SmartErase { x: 0, y: 0, w: 10, h: 10, iterations: 0 };
        se.translate(7.0, 8.0);
        assert_eq!(se.bounds(), Some(Rect { x: 7, y: 8, w: 10, h: 10 }));
    }
}
