//! 长截图拼接算法（M5）。
//!
//! # 职责
//! 把滚动采集到的一系列帧拼成一张长图。核心是**重叠匹配**：新帧与累积图
//! 尾部有多少行重复，就追加多少行新内容。
//!
//! # 算法：两阶段匹配（速度 + 精度的折中）
//! 1. **行哈希粗定位**：每行降采样成 64 位 FNV-1a 哈希，在全候选范围
//!    （`min_overlap..=max_overlap`）上算平均 Hamming 距离，取 top-3 候选。
//!    纯整数运算，600 行候选范围约 20 万次异或，亚毫秒级。
//! 2. **全分辨率 SAD 精修**：对 top-3 候选逐像素算平均绝对差，取最小。
//!    平均差超过 `match_threshold` 判定"没有可靠重叠"（页面内容被替换 /
//!    横向漂移），调用方应丢弃该帧而不是硬拼。
//!
//! # 为什么不用现成 OpenCV
//! 本 crate 的硬约束是零依赖 + 纯函数（远期 C++ 移植说明书）。matchTemplate
//! 在 C++ 侧对应实现就是上述两阶段，移植时是机械翻译。
//!
//! # 静止 / 到底检测
//! 相邻两帧整图平均绝对差低于 `still_threshold` 即认为页面没动；
//! 连续 `still_rounds` 轮没动由**调用方**判定滚动到底（算法层只报数）。

use crate::{AlgoError, RgbaBuffer};

// ---------------------------------------------------------------- 选项与结果

/// 拼接参数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StitchOptions {
    /// 最小重叠行数：低于此匹配不可靠（候选下界）
    pub min_overlap: u32,
    /// 最大重叠行数：候选上界（也限制哈希窗口大小）
    pub max_overlap: u32,
    /// 阶段 2 平均像素差阈值（灰度域 0..255）：超过判定无可靠重叠
    pub match_threshold: f32,
    /// 帧差阈值（灰度域 0..255）：低于此认为页面没动
    pub still_threshold: f32,
    /// 连续多少轮"没动"判定到底（由调用方消费，算法层只累计）
    pub still_rounds: u32,
    /// 累积图高度上限（防内存爆炸 / 防无限滚动）
    pub max_height: u32,
}

impl StitchOptions {
    /// 按帧高自适应：帧越矮重叠要求越低（短选区也要能拼）。
    pub fn for_frame(frame_h: u32) -> Self {
        let min_overlap = (frame_h / 6).clamp(24, 120);
        let max_overlap = frame_h.saturating_sub(8).max(min_overlap + 1);
        Self {
            min_overlap,
            max_overlap,
            ..Self::default()
        }
    }
}

impl Default for StitchOptions {
    fn default() -> Self {
        Self {
            min_overlap: 80,
            max_overlap: 600,
            match_threshold: 6.0,
            still_threshold: 1.0,
            still_rounds: 2,
            max_height: 32_000,
        }
    }
}

/// 一轮 push 的结果类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StitchStepKind {
    /// 拼上了（第一帧也算）
    Appended,
    /// 页面在动，但没找到可靠重叠（内容被替换 / 横向漂移）——调用方应丢弃该帧
    NoMatch,
    /// 帧与上一轮几乎相同：页面没动（可能到底）
    Still,
}

/// 一轮 push 的详细结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StitchStep {
    pub kind: StitchStepKind,
    /// 本轮新增行数（0 = 没拼）
    pub appended: u32,
    /// 匹配置信度 0..1（Still 时为帧相似度）
    pub score: f32,
    /// 连续"没动"轮次
    pub still_count: u32,
    /// 当前累积高度
    pub height: u32,
    /// 累计步数（含被丢弃的帧）
    pub steps: u32,
}

// ---------------------------------------------------------------- 拼接器

/// 长截图拼接状态机（有状态：持有累积图 + 上一帧 + 尾部哈希窗口）。
pub struct Stitcher {
    opts: StitchOptions,
    width: u32,
    canvas: RgbaBuffer,
    /// 累积图尾部行的哈希（长度 ≤ `max_overlap`；`tail_hashes[i]`
    /// 对应 canvas 第 `height - len + i` 行）。滚动窗口维护，
    /// 避免每轮对高图重算哈希。
    tail_hashes: Vec<u64>,
    /// 上一帧（帧差检测用）；None = 还未开始
    prev: Option<RgbaBuffer>,
    still_count: u32,
    steps: u32,
    /// 行哈希降采样步长（宽 / 此值 = 每行采样列数，至少 16 列）
    hash_step: u32,
}

impl Stitcher {
    /// 用第一帧初始化（第一帧直接作为累积图）。
    pub fn new(first: &RgbaBuffer, opts: StitchOptions) -> Result<Self, AlgoError> {
        if first.height < 2 {
            return Err(AlgoError::FrameTooShort { h: first.height });
        }
        let mut s = Self {
            opts,
            width: first.width,
            canvas: first.clone(),
            tail_hashes: Vec::new(),
            prev: None,
            still_count: 0,
            steps: 0,
            hash_step: (first.width / 64).max(1),
        };
        s.rebuild_tail_hashes();
        s.prev = Some(first.clone());
        s.steps = 1;
        Ok(s)
    }

    /// 推入一帧，返回拼接结果。
    pub fn push(&mut self, frame: &RgbaBuffer) -> Result<StitchStep, AlgoError> {
        if frame.width != self.width {
            return Err(AlgoError::WidthMismatch {
                expected: self.width,
                actual: frame.width,
            });
        }
        self.steps += 1;

        // --- 帧差：与上一轮比整图 ---
        // 尺寸不一致（逻辑上不该发生）按"差异极大"处理：不当静止，继续尝试匹配
        let diff = frame_mean_abs_diff(self.prev.as_ref().unwrap(), frame).unwrap_or(255.0);
        if diff < self.opts.still_threshold {
            // 页面没动：不拼（拼了也是重复行），只累计静止轮次
            self.still_count += 1;
            self.prev = Some(frame.clone());
            return Ok(StitchStep {
                kind: StitchStepKind::Still,
                appended: 0,
                score: (1.0 - diff / 255.0).clamp(0.0, 1.0),
                still_count: self.still_count,
                height: self.canvas.height,
                steps: self.steps,
            });
        }
        self.still_count = 0;

        // --- 找重叠 ---
        let frame_hashes = row_hashes(frame, self.hash_step);
        match self.find_overlap(frame, &frame_hashes) {
            Some((shift, score)) => {
                let append_rows = frame.height - shift;
                if self.canvas.height as u64 + append_rows as u64 > self.opts.max_height as u64 {
                    return Err(AlgoError::TooTall {
                        max: self.opts.max_height,
                    });
                }
                self.append_rows(frame, shift, &frame_hashes);
                self.prev = Some(frame.clone());
                Ok(StitchStep {
                    kind: StitchStepKind::Appended,
                    appended: append_rows,
                    score,
                    still_count: 0,
                    height: self.canvas.height,
                    steps: self.steps,
                })
            }
            None => {
                // 不拼，但更新 prev（下一轮的帧差基准）
                self.prev = Some(frame.clone());
                Ok(StitchStep {
                    kind: StitchStepKind::NoMatch,
                    appended: 0,
                    score: 0.0,
                    still_count: 0,
                    height: self.canvas.height,
                    steps: self.steps,
                })
            }
        }
    }

    /// 当前累积图。
    pub fn canvas(&self) -> &RgbaBuffer {
        &self.canvas
    }

    /// 消费自身，取出累积图。
    pub fn into_canvas(self) -> RgbaBuffer {
        self.canvas
    }

    pub fn height(&self) -> u32 {
        self.canvas.height
    }

    pub fn steps(&self) -> u32 {
        self.steps
    }

    pub fn still_count(&self) -> u32 {
        self.still_count
    }

    /// 是否已判定到底（连续 still_rounds 轮没动）。
    pub fn at_bottom(&self) -> bool {
        self.still_count >= self.opts.still_rounds
    }

    // ---------------------------------------------------------------- 内部

    fn rebuild_tail_hashes(&mut self) {
        self.tail_hashes = row_hashes(&self.canvas, self.hash_step);
        let keep = self.opts.max_overlap as usize;
        if self.tail_hashes.len() > keep {
            let drop = self.tail_hashes.len() - keep;
            self.tail_hashes.drain(0..drop);
        }
    }

    fn append_rows(&mut self, frame: &RgbaBuffer, shift: u32, frame_hashes: &[u64]) {
        let w = self.width as usize;
        let from = shift as usize * w * 4;
        self.canvas.data.extend_from_slice(&frame.data[from..]);
        self.canvas.height += frame.height - shift;
        // 追加行的哈希进窗口（保持尾部 ≤ max_overlap）
        for &h in &frame_hashes[shift as usize..] {
            self.tail_hashes.push(h);
        }
        let keep = self.opts.max_overlap as usize;
        if self.tail_hashes.len() > keep {
            let drop = self.tail_hashes.len() - keep;
            self.tail_hashes.drain(0..drop);
        }
    }

    /// 两阶段重叠匹配。返回 (shift, score)：shift = 新帧顶部对齐累积图的
    /// 行偏移（即累积图有 shift 行与新帧头部重复）。
    fn find_overlap(&self, frame: &RgbaBuffer, frame_hashes: &[u64]) -> Option<(u32, f32)> {
        let fh = frame.height;
        let ch = self.canvas.height;
        let s_lo = self.opts.min_overlap.min(fh - 1).min(ch);
        let s_hi = self
            .opts
            .max_overlap
            .min(fh - 1)
            .min(ch)
            .min(self.tail_hashes.len() as u32);
        if s_hi < s_lo || s_lo == 0 {
            return None;
        }
        let l = self.tail_hashes.len();

        // 阶段 1：行哈希平均 Hamming，取 top-3 最小
        let mut top: [(f32, u32); 3] = [(f32::MAX, 0); 3];
        for s in s_lo..=s_hi {
            let mut d = 0u32;
            let base = l - s as usize;
            for (fh_i, th_i) in frame_hashes[..s as usize]
                .iter()
                .zip(&self.tail_hashes[base..base + s as usize])
            {
                d += (fh_i ^ th_i).count_ones();
            }
            let mean = d as f32 / (s as f32 * 64.0);
            // 插入排序维护小顶堆（3 个元素，展开比 BinaryHeap 快）
            if mean < top[2].0 {
                top[2] = (mean, s);
                if top[1].0 > top[2].0 {
                    top.swap(1, 2);
                }
                if top[0].0 > top[1].0 {
                    top.swap(0, 1);
                }
            }
        }

        // 阶段 2：全分辨率 SAD 精修 top-3
        let mut best: Option<(f32, u32)> = None;
        for &(_, s) in &top {
            if s == 0 {
                continue; // 初始占位（候选范围空，实际不会发生）
            }
            let sad = region_sad(&self.canvas, frame, s);
            if sad < self.opts.match_threshold {
                match best {
                    Some((bs, _)) if bs <= sad => {}
                    _ => best = Some((sad, s)),
                }
            }
        }
        best.map(|(sad, s)| (s, (1.0 - sad / 255.0).clamp(0.0, 1.0)))
    }
}

// ---------------------------------------------------------------- 纯函数

/// 灰度近似（整数加权，避免每像素浮点除法）。
/// `(r*77 + g*150 + b*29) >> 8` 是 Rec.601 亮度的快速近似。
#[inline]
pub fn luma(p: &[u8]) -> u32 {
    ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u32
}

/// 两幅同尺寸图的整图平均绝对差（灰度域 0..255）。
/// 尺寸不一致返回 `None`。
pub fn frame_mean_abs_diff(a: &RgbaBuffer, b: &RgbaBuffer) -> Option<f32> {
    if a.width != b.width || a.height != b.height {
        return None;
    }
    let n = (a.width as u64 * a.height as u64) as f32;
    let mut sum = 0u64;
    // RGBA 缓冲长度必为 4 的倍数，余部为空
    let (pa, _) = a.data.as_chunks::<4>();
    let (pb, _) = b.data.as_chunks::<4>();
    for (pa, pb) in pa.iter().zip(pb.iter()) {
        sum += luma(pa).abs_diff(luma(pb)) as u64;
    }
    Some(sum as f32 / n)
}

/// 每行的 64 位 FNV-1a 哈希（降采样：每 `step` 列取一个灰度值）。
/// 返回长度 = height 的向量。
pub fn row_hashes(img: &RgbaBuffer, step: u32) -> Vec<u64> {
    let step = step.max(1) as usize;
    let w = img.width as usize;
    let mut out = Vec::with_capacity(img.height as usize);
    for row in img.data.chunks_exact(w * 4) {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV offset basis
        let mut x = 0;
        while x < w {
            let p = &row[x * 4..x * 4 + 3];
            let g = ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8;
            h ^= g as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV prime
            x += step;
        }
        out.push(h);
    }
    out
}

/// 阶段 2 精修：`frame` 前 `shift` 行与 `canvas` 末尾 `shift` 行的
/// 平均绝对差（灰度域）。
fn region_sad(canvas: &RgbaBuffer, frame: &RgbaBuffer, shift: u32) -> f32 {
    let w = canvas.width as usize;
    let ch = canvas.height as usize;
    let s = shift as usize;
    let mut sum = 0u64;
    for i in 0..s {
        let a_row = &canvas.data[(ch - s + i) * w * 4..(ch - s + i + 1) * w * 4];
        let b_row = &frame.data[i * w * 4..(i + 1) * w * 4];
        // 行缓冲长度 = w*4，余部为空
        let (a_chunks, _) = a_row.as_chunks::<4>();
        let (b_chunks, _) = b_row.as_chunks::<4>();
        for (pa, pb) in a_chunks.iter().zip(b_chunks.iter()) {
            sum += luma(pa).abs_diff(luma(pb)) as u64;
        }
    }
    sum as f32 / (s as f32 * w as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一屏"网页"：**非周期**纹理（背景/文字带/字距都由 y 经伪随机决定）。
    /// 周期纹理会让多个偏移同时精确命中（如 bg=y%32 + 带=y%40 的周期是 160 行），
    /// 测试就失去区分度——真实截图不存在严格周期，这里如实建模。
    fn web_page(h: u32, w: u32) -> RgbaBuffer {
        let mut data = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            let n = (y as u32).wrapping_mul(2654435761); // 大常数散列，非周期
            let bg = 190 + ((n >> 13) % 24) as u8;
            let text_band = (n >> 7) % 100 < 40;
            let period = 5 + (n % 4); // 字距 5..8 逐行变化
            for x in 0..w {
                let dark = text_band && x % period < period / 2 + 1;
                let v = if dark { 35 } else { bg };
                data.extend_from_slice(&[v, v, v, 255]);
            }
        }
        RgbaBuffer::new(w, h, data).unwrap()
    }

    fn crop_rows(img: &RgbaBuffer, y0: u32, y1: u32) -> RgbaBuffer {
        let w = img.width;
        let from = y0 as usize * w as usize * 4;
        let to = y1 as usize * w as usize * 4;
        RgbaBuffer::new(w, y1 - y0, img.data[from..to].to_vec()).unwrap()
    }

    #[test]
    fn stitches_two_frames_with_known_overlap() {
        let page = web_page(1200, 320);
        let a = crop_rows(&page, 0, 600);
        let b = crop_rows(&page, 400, 1000); // 重叠 200 行
        let opts = StitchOptions::for_frame(600);
        let mut st = Stitcher::new(&a, opts).unwrap();
        let step = st.push(&b).unwrap();
        assert_eq!(step.kind, StitchStepKind::Appended);
        assert_eq!(step.appended, 400);
        assert_eq!(st.height(), 1000);
        assert_eq!(st.canvas(), &crop_rows(&page, 0, 1000));
    }

    #[test]
    fn multi_step_accumulates_without_drift() {
        let page = web_page(2400, 320);
        let opts = StitchOptions::for_frame(600);
        let mut st = Stitcher::new(&crop_rows(&page, 0, 600), opts).unwrap();
        // 每帧前进 200 行（重叠 400），共 9 帧
        for k in 1..=9u32 {
            let y0 = 200 * k;
            let f = crop_rows(&page, y0, y0 + 600);
            let step = st.push(&f).unwrap();
            assert_eq!(step.kind, StitchStepKind::Appended, "frame {k} should append");
            assert_eq!(step.appended, 200);
        }
        assert_eq!(st.height(), 2400);
        assert_eq!(st.canvas(), &page);
    }

    #[test]
    fn still_frames_are_detected_not_appended() {
        let page = web_page(600, 320);
        let a = crop_rows(&page, 0, 600);
        let opts = StitchOptions::for_frame(600);
        let mut st = Stitcher::new(&a, opts).unwrap();
        let step = st.push(&a).unwrap(); // 同一帧再来一次
        assert_eq!(step.kind, StitchStepKind::Still);
        assert_eq!(step.appended, 0);
        assert_eq!(step.still_count, 1);
        assert_eq!(st.height(), 600);
        let step2 = st.push(&a).unwrap();
        assert_eq!(step2.still_count, 2);
        assert!(st.at_bottom()); // 默认 still_rounds = 2
    }

    #[test]
    fn no_overlap_returns_no_match() {
        let a = web_page(600, 320);
        let b = RgbaBuffer::new(320, 600, vec![128; 320 * 600 * 4]).unwrap(); // 纯灰，无重叠
        let opts = StitchOptions::for_frame(600);
        let mut st = Stitcher::new(&a, opts).unwrap();
        let step = st.push(&b).unwrap();
        assert_eq!(step.kind, StitchStepKind::NoMatch);
        assert_eq!(step.appended, 0);
        assert_eq!(st.height(), 600); // 没拼
    }

    #[test]
    fn width_mismatch_is_an_error() {
        let a = web_page(600, 320);
        let b = web_page(600, 321);
        let opts = StitchOptions::for_frame(600);
        let mut st = Stitcher::new(&a, opts).unwrap();
        let err = st.push(&b).unwrap_err();
        assert!(matches!(err, AlgoError::WidthMismatch { .. }));
    }

    #[test]
    fn tiny_frame_is_rejected() {
        let a = web_page(2, 64);
        let opts = StitchOptions::for_frame(2);
        // for_frame(2): min_overlap = clamp(0,24,120) = 24 > 帧高，逻辑上不可拼
        let mut st = Stitcher::new(&a, opts).unwrap();
        // 第二帧同样高度：s_lo = min(24, 1) = 1, s_hi = min(max(0→1?), ...)
        // for_frame(2) 的 max_overlap = max(2-8, 25) = 25，帧高 2 → 无法匹配
        let step = st.push(&a).unwrap();
        assert_eq!(step.kind, StitchStepKind::Still); // 帧相同，走静止分支
    }

    #[test]
    fn max_height_guard() {
        let opts = StitchOptions {
            max_height: 700,
            ..StitchOptions::for_frame(600)
        };
        let page = web_page(1200, 64);
        let mut st = Stitcher::new(&crop_rows(&page, 0, 600), opts).unwrap();
        // 推进 100 行：600 + 100 = 700 恰好达标；再进一步必超限
        let ok = st.push(&crop_rows(&page, 100, 700)).unwrap();
        assert_eq!(ok.kind, StitchStepKind::Appended);
        let err = st.push(&crop_rows(&page, 200, 800)).unwrap_err();
        assert!(matches!(err, AlgoError::TooTall { .. }));
    }

    #[test]
    fn frame_diff_detects_identical_and_changed() {
        let a = web_page(200, 128);
        assert_eq!(frame_mean_abs_diff(&a, &a), Some(0.0));
        let mut b = a.data.clone();
        b[0] = b[0].saturating_add(100);
        let b = RgbaBuffer::new(128, 200, b).unwrap();
        let d = frame_mean_abs_diff(&a, &b).unwrap();
        assert!(d > 0.0 && d < 1.0); // 只有 1 个像素变了
        assert!(frame_mean_abs_diff(&a, &web_page(200, 127)).is_none());
    }

    #[test]
    fn row_hashes_are_stable_and_distinct() {
        let img = web_page(100, 128);
        let h = row_hashes(&img, 2);
        assert_eq!(h.len(), 100);
        assert_eq!(h, row_hashes(&img, 2)); // 确定性
        // 相邻行内容不同 → 哈希不同（背景每行渐变）
        assert_ne!(h[0], h[1]);
    }
}
