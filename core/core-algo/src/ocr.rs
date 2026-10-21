//! OCR 纯算法层：DBNet 检测后处理 + CTC 解码 + 预处理尺寸规划。
//!
//! # 与 PaddleOCR 的对应关系
//!
//! 官方后处理（`db_postprocess.py` / `rec_postprocess.py`）在本模块有逐步骤
//! 的纯函数实现，便于将来机械移植到 C++（鸿蒙 PC 路线）：
//!
//! | PaddleOCR | 本模块 |
//! |---|---|
//! | `resize_img_type0` | [`det_input_size`] |
//! | `resize_normize_img` 的尺寸部分 | [`rec_input_size`] |
//! | `boxes_from_bitmap`（阈值 + 连通域 + 打分） | [`detect_text_boxes`] |
//! | `unclip`（按 `area*ratio/perimeter` 逐框外扩） | [`detect_text_boxes`] 内部 |
//! | `CTCLabelDecode` | [`ctc_decode`] |
//!
//! # 与官方实现的差异（有意的简化）
//!
//! - 官方用 `cv2.findContours` + `minAreaRect` 得到**旋转框**；本模块用
//!   连通域 AABB。截图场景 95% 是水平文本，AABB 足够；旋转文本见
//!   `docs/ROADMAP.md` 的后续项（需要凸包 + 旋转卡壳 + 透视变换）。
//! - 官方的 `unclip` 是多边形偏移（ClipperLib）；本模块用「逐连通域
//!   按各自距离做棋盘膨胀」近似——距离公式同源（`area*ratio/perimeter`），
//!   只把几何操作换成像素操作。
//! - 二者共同点：**膨胀半径逐连通域自适应**。这是不把相邻行并成一个框的
//!   关键——整行一个大连通域时 perimeter 大 → 半径小；字符碎裂成小连通域时
//!   perimeter 小 → 半径大，正好把同行字符粘起来。

/// 检测后处理参数（对齐 PP-OCRv4 mobile 的常用取值，可按实测调整）。
#[derive(Debug, Clone, Copy)]
pub struct DetOptions {
    /// 概率图二值化阈值（PaddleOCR `det_db_thresh`）
    pub thresh: f32,
    /// 连通域平均概率下限（`det_db_box_thresh`）
    pub box_thresh: f32,
    /// 外扩比例（`det_db_unclip_ratio`）
    pub unclip_ratio: f32,
    /// 框最短边下限（检测空间像素，`min_size`）
    pub min_size: f32,
    /// 参与后处理的连通域数量上限（防止碎裂概率图拖垮性能）
    pub max_candidates: usize,
    /// 单个连通域外扩半径上限（像素）
    pub max_unclip: f32,
}

impl Default for DetOptions {
    fn default() -> Self {
        Self {
            thresh: 0.3,
            box_thresh: 0.5,
            unclip_ratio: 2.0,
            min_size: 3.0,
            max_candidates: 1000,
            max_unclip: 24.0,
        }
    }
}

/// 检测输出的文本框（检测空间坐标，`max_*` 为**含**的像素坐标）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetBox {
    pub min_x: f32,
    pub min_y: f32,
    pub max_x: f32,
    pub max_y: f32,
    /// 框内概率均值（0..1）
    pub score: f32,
}

impl DetBox {
    pub fn width(&self) -> f32 {
        self.max_x - self.min_x + 1.0
    }
    pub fn height(&self) -> f32 {
        self.max_y - self.min_y + 1.0
    }
}

/// 概率图二值化：`1` = 文本像素。
///
/// `probs.len()` 必须等于 `width * height`，否则 panic（纯算法层对调用方
/// 契约违约直接失败，不做静默容错）。
pub fn binarize(probs: &[f32], width: usize, height: usize, thresh: f32) -> Vec<u8> {
    assert_eq!(
        probs.len(),
        width * height,
        "probs length {} != width*height {}",
        probs.len(),
        width * height
    );
    probs
        .iter()
        .map(|&p| if p > thresh { 1 } else { 0 })
        .collect()
}

/// 连通域（8-连通，与 `cv2.findContours` 的连通性一致）。
#[derive(Debug, Clone)]
pub struct Component {
    pub min_x: usize,
    pub min_y: usize,
    pub max_x: usize,
    pub max_y: usize,
    /// 域内全部前景像素（检测空间坐标）
    pub pixels: Vec<(u32, u32)>,
}

/// 8-连通标记。返回顺序即扫描顺序（从上到下、从左到右）。
pub fn label_components(bitmap: &[u8], width: usize, height: usize) -> Vec<Component> {
    label_components_inner(bitmap, width, height).0
}

/// [`label_components`] 的内部版：额外返回标签图（`u32::MAX` = 背景）。
///
/// 标签图是算「框内墨迹概率均值」的关键：unclip 外扩后的连通域混入了大量
/// 背景像素，直接对扩后区域求均值会把置信度稀释到 0.3 以下（扩得越狠
/// 稀释越狠），必须回到**原图墨迹像素**上算。
fn label_components_inner(
    bitmap: &[u8],
    width: usize,
    height: usize,
) -> (Vec<Component>, Vec<u32>) {
    assert_eq!(bitmap.len(), width * height, "bitmap length mismatch");
    let mut visited = vec![false; bitmap.len()];
    let mut labels = vec![u32::MAX; bitmap.len()];
    let mut out: Vec<Component> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    const NEIGHBORS: [(isize, isize); 8] = [
        (-1, 0),
        (1, 0),
        (0, -1),
        (0, 1),
        (-1, -1),
        (-1, 1),
        (1, -1),
        (1, 1),
    ];
    for y in 0..height {
        for x in 0..width {
            let i = y * width + x;
            if bitmap[i] == 0 || visited[i] {
                continue;
            }
            let label = out.len() as u32;
            visited[i] = true;
            stack.push((x, y));
            let (mut min_x, mut min_y, mut max_x, mut max_y) = (x, y, x, y);
            let mut pixels: Vec<(u32, u32)> = Vec::new();
            while let Some((cx, cy)) = stack.pop() {
                labels[cy * width + cx] = label;
                pixels.push((cx as u32, cy as u32));
                min_x = min_x.min(cx);
                min_y = min_y.min(cy);
                max_x = max_x.max(cx);
                max_y = max_y.max(cy);
                for (dx, dy) in NEIGHBORS {
                    let nx = cx as isize + dx;
                    let ny = cy as isize + dy;
                    if nx < 0 || ny < 0 || nx >= width as isize || ny >= height as isize {
                        continue;
                    }
                    let ni = ny as usize * width + nx as usize;
                    if bitmap[ni] != 0 && !visited[ni] {
                        visited[ni] = true;
                        stack.push((nx as usize, ny as usize));
                    }
                }
            }
            out.push(Component {
                min_x,
                min_y,
                max_x,
                max_y,
                pixels,
            });
        }
    }
    (out, labels)
}

/// 参与 unclip 的连通域：像素 + 边界像素（4-邻域有背景/出界的像素）。
///
/// `boundary` 单独存一份是有意为之：对一个实心区域做半径 d 的棋盘膨胀，
/// 等价于对其边界像素各盖一个 (2d+1)² 的方块（区域内任意像素到边界的
/// 路径上必经过边界，由度量三角不等式可证），可省掉大量重复盖章。
#[derive(Debug, Clone)]
struct UnclipBlob {
    min_x: usize,
    min_y: usize,
    max_x: usize,
    max_y: usize,
    area: u32,
    /// 平均概率（域内像素在概率图上的均值）
    score: f32,
    /// 域内全部像素（用于扩后框的分数溯源）
    pixels: Vec<(u32, u32)>,
    /// 边界像素坐标
    boundary: Vec<(u32, u32)>,
}

fn to_unclip_blob(
    c: &Component,
    bitmap: &[u8],
    probs: &[f32],
    width: usize,
    height: usize,
) -> UnclipBlob {
    let mut sum = 0.0f32;
    let mut boundary = Vec::new();
    for &(x, y) in &c.pixels {
        let (xu, yu) = (x as usize, y as usize);
        sum += probs[yu * width + xu];
        let on_edge = xu == 0
            || yu == 0
            || xu + 1 == width
            || yu + 1 == height
            || bitmap[yu * width + xu - 1] == 0
            || bitmap[yu * width + xu + 1] == 0
            || bitmap[(yu - 1) * width + xu] == 0
            || bitmap[(yu + 1) * width + xu] == 0;
        if on_edge {
            boundary.push((x, y));
        }
    }
    UnclipBlob {
        min_x: c.min_x,
        min_y: c.min_y,
        max_x: c.max_x,
        max_y: c.max_y,
        area: c.pixels.len() as u32,
        score: sum / c.pixels.len() as f32,
        pixels: c.pixels.clone(),
        boundary,
    }
}

/// 逐连通域 unclip：按 DBNet 公式 `distance = area * ratio / perimeter`
/// 估算外扩半径并做棋盘膨胀，返回膨胀后的位图。
fn unclip_expand(
    blobs: &[UnclipBlob],
    width: usize,
    height: usize,
    ratio: f32,
    max_dist: f32,
) -> Vec<u8> {
    let mut out = vec![0u8; width * height];
    let w = width as i32;
    let h = height as i32;
    for b in blobs {
        let d = if b.boundary.is_empty() {
            1.0
        } else {
            (b.area as f32 * ratio / b.boundary.len() as f32).round()
        };
        let d = d.clamp(1.0, max_dist) as i32;
        for &(x, y) in &b.boundary {
            let x0 = (x as i32 - d).max(0);
            let x1 = (x as i32 + d).min(w - 1);
            let y0 = (y as i32 - d).max(0);
            let y1 = (y as i32 + d).min(h - 1);
            for yy in y0..=y1 {
                let row = yy as usize * width;
                for xx in x0..=x1 {
                    out[row + xx as usize] = 1;
                }
            }
        }
    }
    out
}

/// 按阅读顺序排序：先分行（垂直方向有实质重叠视为同一行），行内按 x。
///
/// 为什么不能直接按 `(min_y, min_x)` 排序：双栏截图里右栏第一行的 y 往往
/// 小于左栏第二行，直接排序会把两栏内容交错，「复制全部」出来是乱的。
pub fn sort_reading_order(boxes: &mut Vec<DetBox>) {
    boxes.sort_by(|a, b| {
        a.min_y
            .partial_cmp(&b.min_y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.min_x.partial_cmp(&b.min_x).unwrap_or(std::cmp::Ordering::Equal))
    });
    let mut lines: Vec<Vec<DetBox>> = Vec::new();
    for b in boxes.drain(..) {
        if let Some(last) = lines.last_mut() {
            let ly0 = last.iter().map(|x| x.min_y).fold(f32::INFINITY, f32::min);
            let ly1 = last.iter().map(|x| x.max_y).fold(f32::NEG_INFINITY, f32::max);
            let overlap = (ly1.min(b.max_y) - ly0.max(b.min_y) + 1.0).max(0.0);
            if overlap > 0.5 * b.height().min(ly1 - ly0 + 1.0) {
                last.push(b);
                continue;
            }
        }
        lines.push(vec![b]);
    }
    for line in &mut lines {
        line.sort_by(|a, b| {
            a.min_x
                .partial_cmp(&b.min_x)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    *boxes = lines.into_iter().flatten().collect();
}

/// DBNet 检测后处理一站式入口：概率图 → 有序文本框列表。
pub fn detect_text_boxes(
    probs: &[f32],
    width: usize,
    height: usize,
    opts: &DetOptions,
) -> Vec<DetBox> {
    assert_eq!(probs.len(), width * height, "probs length mismatch");
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let bitmap = binarize(probs, width, height, opts.thresh);
    let comps = label_components(&bitmap, width, height);
    let mut blobs: Vec<UnclipBlob> = comps
        .iter()
        .map(|c| to_unclip_blob(c, &bitmap, probs, width, height))
        .collect();
    // 官方顺序也是先按 min_size / box_thresh 过滤，再 unclip
    blobs.retain(|b| {
        let w = (b.max_x - b.min_x + 1) as f32;
        let h = (b.max_y - b.min_y + 1) as f32;
        w >= opts.min_size && h >= opts.min_size && b.score >= opts.box_thresh
    });
    if blobs.len() > opts.max_candidates {
        blobs.sort_by(|a, b| b.area.cmp(&a.area));
        blobs.truncate(opts.max_candidates);
    }
    if blobs.is_empty() {
        return Vec::new();
    }
    let expanded = unclip_expand(&blobs, width, height, opts.unclip_ratio, opts.max_unclip);
    let (comps2, labels2) = label_components_inner(&expanded, width, height);
    // 框的分数 = 落在框内的**原图墨迹像素**的概率均值。
    // 不能对扩后的区域求均值：外扩壳全是背景，会把置信度稀释到 0.3 以下。
    let mut sums = vec![0.0f32; comps2.len()];
    let mut counts = vec![0u32; comps2.len()];
    for b in &blobs {
        for &(x, y) in &b.pixels {
            let l = labels2[y as usize * width + x as usize];
            if l != u32::MAX {
                let li = l as usize;
                sums[li] += probs[y as usize * width + x as usize];
                counts[li] += 1;
            }
        }
    }
    let mut boxes: Vec<DetBox> = comps2
        .iter()
        .enumerate()
        .filter(|(i, c)| {
            let w = (c.max_x - c.min_x + 1) as f32;
            let h = (c.max_y - c.min_y + 1) as f32;
            w >= opts.min_size && h >= opts.min_size && counts[*i] > 0
        })
        .map(|(i, c)| DetBox {
            min_x: c.min_x as f32,
            min_y: c.min_y as f32,
            max_x: c.max_x as f32,
            max_y: c.max_y as f32,
            score: sums[i] / counts[i] as f32,
        })
        .collect();
    sort_reading_order(&mut boxes);
    boxes
}

/// 检测端输入尺寸规划（PaddleOCR `resize_img_type0` 等价实现）。
///
/// `limit_type='min'`：短边小于 `limit` 时**放大**到 `limit`（小字检测的
/// 关键），长边不限；随后把两个方向都向上取整到 `multiple` 的倍数。
/// 返回 `(宽, 高)`。
pub fn det_input_size(width: u32, height: u32, limit: u32, multiple: u32) -> (u32, u32) {
    assert!(multiple > 0, "multiple must be > 0");
    let min_side = width.min(height).max(1);
    let ratio = if min_side < limit {
        limit as f32 / min_side as f32
    } else {
        1.0
    };
    let nw = ((width as f32 * ratio).round() as u32).max(1);
    let nh = ((height as f32 * ratio).round() as u32).max(1);
    let round_up = |v: u32| -> u32 {
        let m = multiple;
        ((v + m - 1) / m * m).max(m)
    };
    (round_up(nw), round_up(nh))
}

/// 识别端输入宽度规划：高固定 `height_fixed`，宽按比例且不超过 `max_width`。
///
/// 与 PaddleOCR `resize_normize_img` 一致：超宽时**不保比例**直接压到
/// `max_width`（识别模型对轻微挤压不敏感，保比例反而要缩放高度）。
pub fn rec_input_size(width: u32, height: u32, height_fixed: u32, max_width: u32) -> u32 {
    let h = height.max(1);
    let ratio = width as f32 / h as f32;
    let rw = (height_fixed as f32 * ratio).ceil() as u32;
    rw.clamp(1, max_width.max(1))
}

/// CTC 贪心解码（PaddleOCR `CTCLabelDecode` 等价实现）。
///
/// # 索引约定（PP-OCR 全系统一）
///
/// - `0` = blank（CTC 空洞，不输出）
/// - `1..=dict.len()` = 字典字符（字典文件按行顺序）
/// - `dict.len() + 1` = 空格（PaddleOCR `use_space_char=True` 追加在字典末尾）
///
/// `probs` 为 `[T, C]` 行优先（batch=1 已省略）。返回 `(文本, 置信度)`，
/// 置信度 = 各时刻最大概率的均值。
pub fn ctc_decode(probs: &[f32], time_steps: usize, num_classes: usize, dict: &[String]) -> (String, f32) {
    assert_eq!(
        probs.len(),
        time_steps * num_classes,
        "probs length {} != T*C {}",
        probs.len(),
        time_steps * num_classes
    );
    let mut text = String::new();
    let mut conf_sum = 0.0f32;
    let mut prev: Option<usize> = None;
    for t in 0..time_steps {
        let row = &probs[t * num_classes..(t + 1) * num_classes];
        let mut best = 0usize;
        let mut best_v = row[0];
        for (c, &v) in row.iter().enumerate().skip(1) {
            if v > best_v {
                best_v = v;
                best = c;
            }
        }
        conf_sum += best_v;
        // 折叠重复帧 + 跳过 blank
        if best != 0 && prev != Some(best) {
            if best <= dict.len() {
                text.push_str(&dict[best - 1]);
            } else if best == dict.len() + 1 {
                text.push(' ');
            }
            // best > dict.len()+1：模型类别数与字典不符，丢弃该帧
        }
        prev = Some(best);
    }
    let conf = if time_steps == 0 {
        0.0
    } else {
        conf_sum / time_steps as f32
    };
    (text, conf)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 由 (x,y) 列表构造概率图：给定点为 0.9，其余 0.05
    fn prob_map(width: usize, height: usize, cells: &[(usize, usize)]) -> Vec<f32> {
        let mut v = vec![0.05f32; width * height];
        for &(x, y) in cells {
            v[y * width + x] = 0.9;
        }
        v
    }

    fn solid(width: usize, height: usize, x0: usize, y0: usize, w: usize, h: usize) -> Vec<(usize, usize)> {
        let mut v = Vec::new();
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                v.push((x, y));
            }
        }
        v
    }

    // ---------- binarize ----------

    #[test]
    fn binarize_thresholds_correctly() {
        let probs = vec![0.1, 0.3, 0.31, 0.9, 0.0];
        let b = binarize(&probs, 5, 1, 0.3);
        assert_eq!(b, vec![0, 0, 1, 1, 0]);
    }

    #[test]
    #[should_panic(expected = "probs length")]
    fn binarize_rejects_length_mismatch() {
        binarize(&[0.5; 10], 3, 3, 0.3);
    }

    // ---------- label_components ----------

    #[test]
    fn label_finds_two_separated_blobs() {
        let mut cells = solid(4, 4, 0, 0, 3, 3);
        cells.extend(solid(4, 4, 0, 3, 3, 1)); // 注意：这会与上面斜接，改用远距离
        let _ = cells;
        let mut cells = solid(8, 8, 0, 0, 3, 3);
        cells.extend(solid(8, 8, 5, 5, 3, 3));
        let mut map = vec![0u8; 8 * 8];
        for (x, y) in cells {
            map[y * 8 + x] = 1;
        }
        let comps = label_components(&map, 8, 8);
        assert_eq!(comps.len(), 2);
        assert_eq!((comps[0].min_x, comps[0].min_y, comps[0].max_x, comps[0].max_y), (0, 0, 2, 2));
        assert_eq!(comps[0].pixels.len(), 9);
        assert_eq!((comps[1].min_x, comps[1].min_y, comps[1].max_x, comps[1].max_y), (5, 5, 7, 7));
    }

    #[test]
    fn label_diagonal_touch_merges_into_one() {
        let mut map = vec![0u8; 4 * 4];
        map[0] = 1;
        map[1 * 4 + 1] = 1; // (1,1) 与 (0,0) 斜接
        let comps = label_components(&map, 4, 4);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].pixels.len(), 2);
    }

    #[test]
    fn label_corner_touch_merges_into_one() {
        // (0,0) 与 (1,0) 水平相接： obviously 1 个
        let mut map = vec![0u8; 2 * 2];
        map[0] = 1;
        map[1] = 1;
        let comps = label_components(&map, 2, 2);
        assert_eq!(comps.len(), 1);
    }

    #[test]
    fn label_isolated_pixel_is_its_own_component() {
        let mut map = vec![0u8; 3 * 3];
        map[1 * 3 + 1] = 1;
        let comps = label_components(&map, 3, 3);
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].pixels.len(), 1);
        assert_eq!((comps[0].min_x, comps[0].min_y), (1, 1));
    }

    #[test]
    fn label_empty_bitmap_yields_nothing() {
        let comps = label_components(&vec![0u8; 25], 5, 5);
        assert!(comps.is_empty());
    }

    // ---------- unclip（detect_text_boxes 内部行为，经端到端断言） ----------

    #[test]
    fn unclip_expands_each_blob_by_its_own_distance() {
        // 100x60：大块 20x20 @ (10,10)，小块 5x5 @ (60,10)
        // 大块：area=400, boundary=76 → d = 400*2/76 ≈ 10.5 → 11
        // 小块：area=25,  boundary=16 → d = 25*2/16 ≈ 3.1  → 3
        let width = 100usize;
        let height = 60usize;
        let mut map = vec![0u8; width * height];
        for (x, y) in solid(width, height, 10, 10, 20, 20) {
            map[y * width + x] = 1;
        }
        for (x, y) in solid(width, height, 60, 10, 5, 5) {
            map[y * width + x] = 1;
        }
        let comps = label_components(&map, width, height);
        assert_eq!(comps.len(), 2);
        let probs = vec![0.9f32; width * height];
        let blobs: Vec<UnclipBlob> = comps
            .iter()
            .map(|c| to_unclip_blob(c, &map, &probs, width, height))
            .collect();
        let big = blobs.iter().find(|b| b.area == 400).expect("big blob");
        let small = blobs.iter().find(|b| b.area == 25).expect("small blob");
        assert_eq!(big.boundary.len(), 76);
        assert_eq!(small.boundary.len(), 16);

        let out = unclip_expand(&blobs, width, height, 2.0, 24.0);
        // 大块左扩 11：x=10-11=-1 → 夹到 0；(20, 21) 在块内必然为 1
        assert_eq!(out[21 * width + 0], 1, "大块左扩应触到 x=0");
        assert_eq!(out[21 * width + 40], 1, "大块右扩 11 应到 x=40");
        assert_eq!(out[21 * width + 41], 0, "x=41 不应被大块覆盖");
        assert_eq!(out[0 * width + 20], 1, "大块上扩应触到 y=0");
        // 小块：左扩 3 → x=57..；上扩 3 → y=7..
        assert_eq!(out[12 * width + 57], 1, "小块左扩 3 应到 x=57");
        assert_eq!(out[12 * width + 56], 0, "x=56 不应被小块覆盖");
        assert_eq!(out[7 * width + 62], 1, "小块上扩 3 应到 y=7");
        assert_eq!(out[6 * width + 62], 0, "y=6 不应被小块覆盖");
        // 两块之间（x=41..56）不应连通
        let mut merged = label_components(&out, width, height);
        assert_eq!(merged.len(), 2, "两块不应被 unclip 连通");
        merged.sort_by(|a, b| a.min_x.cmp(&b.min_x));
        assert_eq!((merged[0].min_x, merged[0].max_x), (0, 40));
        assert_eq!((merged[1].min_x, merged[1].max_x), (57, 67));
    }

    // ---------- detect_text_boxes 端到端 ----------

    #[test]
    fn detect_merges_close_chars_into_one_box() {
        // 两个 6x6 方块相距 3px（间隙 < 各自 unclip 半径），应并成一个框
        let width = 40usize;
        let height = 20usize;
        let mut cells = solid(width, height, 5, 7, 6, 6);
        cells.extend(solid(width, height, 14, 7, 6, 6));
        let probs = prob_map(width, height, &cells);
        let boxes = detect_text_boxes(&probs, width, height, &DetOptions::default());
        assert_eq!(boxes.len(), 1, "相邻字符应被 unclip 粘成一个框");
        // 每个方块 area=36, boundary=20 → d = 36*2/20 = 3.6 → 4
        // 左块右界 x=10 扩到 14，右块左界 x=14 扩到 10 → 连通
        // 合并框 = 左块左界-4 .. 右块右界+4 = x∈[1,23]，y∈[3,16]
        let b = boxes[0];
        assert_eq!((b.min_x as i32, b.max_x as i32), (1, 23));
        assert_eq!((b.min_y as i32, b.max_y as i32), (3, 16));
        assert!(b.score > 0.5);
    }

    #[test]
    fn detect_keeps_far_blocks_separate() {
        let width = 60usize;
        let height = 20usize;
        let mut cells = solid(width, height, 2, 7, 6, 6);
        cells.extend(solid(width, height, 50, 7, 6, 6));
        let probs = prob_map(width, height, &cells);
        let boxes = detect_text_boxes(&probs, width, height, &DetOptions::default());
        assert_eq!(boxes.len(), 2, "相距很远的块应保持两个框");
    }

    #[test]
    fn detect_filters_low_score_and_tiny_blobs() {
        let width = 30usize;
        let height = 20usize;
        let mut probs = prob_map(width, height, &solid(width, height, 2, 2, 6, 6));
        // 一个 1x1 的高概率点（min_size 过滤）+ 一个低概率小块（box_thresh 过滤）
        probs[15 * width + 15] = 0.9;
        for (x, y) in solid(width, height, 20, 2, 6, 6) {
            probs[y * width + x] = 0.2;
        }
        let boxes = detect_text_boxes(&probs, width, height, &DetOptions::default());
        assert_eq!(boxes.len(), 1, "只剩中间那个 6x6 的块");
        // 该块 area=36, boundary=20 → d=4，框整体外扩 4px：(2,2)-(7,7) → (0,0)-(11,11)
        assert_eq!((boxes[0].min_x as i32, boxes[0].min_y as i32), (0, 0));
        assert_eq!((boxes[0].max_x as i32, boxes[0].max_y as i32), (11, 11));
    }

    #[test]
    fn detect_sorts_in_reading_order() {
        // 模拟双栏：左栏两行、右栏一行（右栏 y 介于左栏两行之间）
        let width = 100usize;
        let height = 60usize;
        let mut cells = solid(width, height, 5, 5, 20, 10); // 左栏第一行
        cells.extend(solid(width, height, 5, 30, 20, 10)); // 左栏第二行
        cells.extend(solid(width, height, 60, 18, 20, 10)); // 右栏第一行
        let probs = prob_map(width, height, &cells);
        let boxes = detect_text_boxes(&probs, width, height, &DetOptions::default());
        assert_eq!(boxes.len(), 3);
        // 每块 area=200, boundary=56 → d=200*2/56≈7.14→7，框整体外扩 7px：
        // 左栏块 x∈[0,31]（5-7 夹到 0，24+7），右栏块 x∈[53,77]
        let xs: Vec<f32> = boxes.iter().map(|b| b.min_x).collect();
        let ys: Vec<f32> = boxes.iter().map(|b| b.min_y).collect();
        // 阅读顺序：左1（y=0）→ 右1（y=11）→ 左2（y=23）
        assert_eq!(xs, vec![0.0, 53.0, 0.0]);
        assert_eq!(ys, vec![0.0, 11.0, 23.0]);
    }

    #[test]
    fn detect_empty_map_returns_empty() {
        let probs = vec![0.01f32; 100];
        let boxes = detect_text_boxes(&probs, 10, 10, &DetOptions::default());
        assert!(boxes.is_empty());
    }

    // ---------- 尺寸规划 ----------

    #[test]
    fn det_input_size_upscales_small_images() {
        // 400x300：短边 300 < 736 → ratio=736/300≈2.453
        // nw=round(981.3)=981 → 向上取 32 的倍数 = 992；nh=736（恰为 32 倍数）
        assert_eq!(det_input_size(400, 300, 736, 32), (992, 736));
    }

    #[test]
    fn det_input_size_keeps_large_images_untouched() {
        // 1920x1080：短边 1080 ≥ 736 → ratio=1，仅做 32 向上取整
        // 1080 不是 32 倍数 → 取到 1088
        assert_eq!(det_input_size(1920, 1080, 736, 32), (1920, 1088));
    }

    #[test]
    fn det_input_size_rounds_up_to_multiple() {
        // 100x100 → ratio 7.36 → 736x736
        assert_eq!(det_input_size(100, 100, 736, 32), (736, 736));
        // 33x33 → ratio 22.3 → 736x736
        assert_eq!(det_input_size(33, 33, 736, 32), (736, 736));
        // 800x600：短边 600<736 → ratio 1.2267 → nw=981→992, nh=736
        assert_eq!(det_input_size(800, 600, 736, 32), (992, 736));
    }

    #[test]
    fn rec_input_size_keeps_aspect_and_caps() {
        // 高 48：100x20 → 宽 = ceil(48*5) = 240
        assert_eq!(rec_input_size(100, 20, 48, 320), 240);
        // 极宽：1000x20 → ceil(48*50)=2400 → 夹到 320
        assert_eq!(rec_input_size(1000, 20, 48, 320), 320);
        // 极窄：5x20 → ceil(48*0.25)=12
        assert_eq!(rec_input_size(5, 20, 48, 320), 12);
        // 退化高度兜底为 1：10x0 → 宽 480 → 仍被 320 夹住
        assert_eq!(rec_input_size(10, 0, 48, 320), 320);
    }

    // ---------- CTC ----------

    #[test]
    fn ctc_decode_collapses_repeats_and_skips_blank() {
        // 字典 ["A","B"," "]，blank=0。序列：A A B 0 B
        // 标准 CTC 语义：先折叠重复帧（A A→A, B…B 中间夹 blank 不算重复）
        // 再去 blank → "A B B" = "ABB"
        let dict: Vec<String> = vec!["A".into(), "B".into(), " ".into()];
        let c = dict.len() + 2; // 5 类
        let mut probs = vec![0.0f32; 5 * c];
        let seq = [1usize, 1, 2, 0, 2];
        for (t, &cls) in seq.iter().enumerate() {
            probs[t * c + cls] = 0.9;
        }
        let (text, conf) = ctc_decode(&probs, 5, c, &dict);
        assert_eq!(text, "ABB");
        assert!((conf - 0.9).abs() < 1e-6);
    }

    #[test]
    fn ctc_decode_adjacent_blank_separates_repeats() {
        // A blank A → "AA"（blank 把两个 A 隔开，不折叠）
        let dict: Vec<String> = vec!["A".into()];
        let c = dict.len() + 2;
        let mut probs = vec![0.0f32; 3 * c];
        probs[0 * c + 1] = 0.9;
        probs[1 * c + 0] = 0.9;
        probs[2 * c + 1] = 0.9;
        let (text, _) = ctc_decode(&probs, 3, c, &dict);
        assert_eq!(text, "AA");
    }

    #[test]
    fn ctc_decode_maps_space_class() {
        let dict: Vec<String> = vec!["A".into(), "B".into()];
        let c = dict.len() + 2; // blank + A + B + space
        let mut probs = vec![0.0f32; 3 * c];
        probs[0 * c + 1] = 0.8; // A
        probs[1 * c + c - 1] = 0.7; // space（最后一类）
        probs[2 * c + 2] = 0.6; // B
        let (text, _) = ctc_decode(&probs, 3, c, &dict);
        assert_eq!(text, "A B");
    }

    #[test]
    fn ctc_decode_all_blank_returns_empty() {
        let dict: Vec<String> = vec!["A".into()];
        let c = dict.len() + 2;
        let mut probs = vec![0.0f32; 4 * c];
        for t in 0..4 {
            probs[t * c] = 0.99;
        }
        let (text, conf) = ctc_decode(&probs, 4, c, &dict);
        assert_eq!(text, "");
        assert!((conf - 0.99).abs() < 1e-6);
    }

    #[test]
    fn ctc_decode_picks_argmax_per_frame() {
        let dict: Vec<String> = vec!["X".into(), "Y".into()];
        let c = 4;
        let mut probs = vec![0.1f32; 2 * c];
        probs[0 * c + 1] = 0.5; // 帧 0 → dict[0] = X
        probs[1 * c + 2] = 0.4; // 帧 1 → dict[1] = Y
        let (text, _) = ctc_decode(&probs, 2, c, &dict);
        assert_eq!(text, "XY");
    }
}
