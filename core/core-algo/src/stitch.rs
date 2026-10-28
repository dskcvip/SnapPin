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
    ///
    /// `max_overlap` 同时也是**哈希窗口长度**（累积图只保留尾部这么多行参与
    /// 匹配），所以必须 ≥ 帧高−1：否则滚动极小时重叠 = 帧高−k 会超出窗口，
    /// 明明在滚却匹配不上（0.1.13 真机三次 NoMatch 的成因之一）。
    pub fn for_frame(frame_h: u32) -> Self {
        let min_overlap = (frame_h / 6).clamp(24, 120);
        let max_overlap = frame_h + 16;
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
    /// 超长模式降采样倍数：1 = 原分辨率；2 = 已整体缩小一半（高度上限翻倍）
    pub scale: u32,
    /// 与上一帧的整图平均绝对差（灰度域 0..255）。
    ///
    /// **诊断用**：页面"没滚"时它 < `still_threshold`；滚太多（一帧跨过整个
    /// 视口）时它会接近纯噪声水平（>80）。0.1.13 真机 NoMatch 连出 5 轮，
    /// 就是靠它区分"事件没送出去"和"送去但量不对"。
    pub diff: f32,
    /// 阶段 1 在**全候选范围**（含 `min_overlap` 以下）找到的最优行偏移，
    /// 0 表示没有任何候选。真实滚动量小于 `min_overlap` 时这里能看到它。
    pub best_shift: u32,
    /// 上面那个候选的平均 Hamming 距离（0..1，越小越像）
    pub best_mean: f32,
}

/// 整数倍盒式降采样（零依赖：core-algo 不吃任何图像库）。
///
/// `k` 必须是 2 的幂（超长模式每次减半）。落单的尾行/尾列直接丢弃——
/// 差 1 行对拼接无影响，比补边省事且不会引入色边。
fn downscale_by(buf: &RgbaBuffer, k: u32) -> Result<RgbaBuffer, AlgoError> {
    let k = k.max(1);
    if k == 1 {
        return Ok(buf.clone());
    }
    let w = (buf.width / k).max(1);
    let h = (buf.height / k).max(1);
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let kk = (k * k) as u32;
    for oy in 0..h {
        for ox in 0..w {
            let mut acc = [0u32; 4];
            for dy in 0..k {
                let sy = oy * k + dy;
                let row = (sy as usize) * (buf.width as usize) * 4;
                for dx in 0..k {
                    let sx = ox * k + dx;
                    let i = row + (sx as usize) * 4;
                    acc[0] += buf.data[i] as u32;
                    acc[1] += buf.data[i + 1] as u32;
                    acc[2] += buf.data[i + 2] as u32;
                    acc[3] += buf.data[i + 3] as u32;
                }
            }
            let o = ((oy as usize) * (w as usize) + ox as usize) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / kk) as u8;
            }
        }
    }
    RgbaBuffer::new(w, h, out)
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
    /// 超长模式：>1 表示整条链路已按此倍数降采样（画布与后续帧一起缩）
    scale: u32,
}

/// 超长模式允许的最大降采样倍数（再大就没意义了：1/8 已经糊到不可读）。
const MAX_LONG_SCALE: u32 = 8;

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
            scale: 1,
        };
        s.rebuild_tail_hashes();
        s.prev = Some(first.clone());
        s.steps = 1;
        Ok(s)
    }

    /// 当前超长模式的降采样倍数（1 = 原分辨率）。
    pub fn scale(&self) -> u32 {
        self.scale
    }

    /// 帧按当前 scale 降采样（超长模式下画布与帧必须同一比例，否则宽度对不上）。
    fn fit_frame(&self, raw: &RgbaBuffer) -> Result<RgbaBuffer, AlgoError> {
        if self.scale <= 1 {
            return Ok(raw.clone());
        }
        downscale_by(raw, self.scale)
    }

    /// 进入/推进超长模式：把画布与参考帧整体缩小一半，scale 翻倍。
    ///
    /// 返回 false = 已经到最大降采样倍数，无法再压（此时才真正报 TooTall）。
    fn shrink(&mut self) -> bool {
        if self.scale >= MAX_LONG_SCALE {
            return false;
        }
        let Ok(half) = downscale_by(&self.canvas, 2) else {
            return false;
        };
        self.canvas = half;
        self.width = self.canvas.width;
        self.hash_step = (self.width / 64).max(1);
        self.scale *= 2;
        // 参考帧必须同步缩，否则下一轮帧差/匹配的尺度不一致
        if let Some(prev) = self.prev.take() {
            self.prev = downscale_by(&prev, 2).ok();
        }
        self.rebuild_tail_hashes();
        true
    }

    /// 推入一帧，返回拼接结果。
    ///
    /// 超长模式：高度累积到 `max_height` 时不再直接报错，而是**整体降采样一半**
    /// 后继续拼（高度上限随之翻倍）。这是长截图工具通行做法——宁可整张清晰度
    /// 掉一半，也不要"滚到一半告诉你拼不下了"。中途收缩时本轮会按新尺度重算，
    /// 保证不破坏已有的拼接结果。
    pub fn push(&mut self, raw: &RgbaBuffer) -> Result<StitchStep, AlgoError> {
        let mut frame = self.fit_frame(raw)?;
        loop {
            match self.push_once(&frame) {
                Err(AlgoError::TooTall { .. }) if self.shrink() => {
                    // 收缩后本轮按新尺度重来（push_once 在报错前没有改动状态）
                    frame = downscale_by(&frame, 2)?;
                    continue;
                }
                other => return other,
            }
        }
    }

    /// 单轮拼接（不含超长模式处理）。TooTall 在此**不改状态**，便于调用方收缩后重试。
    fn push_once(&mut self, frame: &RgbaBuffer) -> Result<StitchStep, AlgoError> {
        if frame.width != self.width {
            return Err(AlgoError::WidthMismatch {
                expected: self.width,
                actual: frame.width,
            });
        }
        self.steps += 1;

        let frame_hashes = row_hashes(frame, self.hash_step);
        // --- 帧差：与上一轮比整图 ---
        // 尺寸不一致（逻辑上不该发生）按"差异极大"处理：不当静止，继续尝试匹配
        let diff = frame_mean_abs_diff(self.prev.as_ref().unwrap(), frame).unwrap_or(255.0);
        // 诊断用的全范围最优候选（含 min_overlap 以下），见 find_overlap 注释
        let (diag_shift, diag_mean) = self.scan_best_shift(&frame_hashes);
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
                scale: self.scale,
                diff,
                best_shift: diag_shift,
                best_mean: diag_mean,
            });
        }
        self.still_count = 0;

        // --- 找重叠 ---
        match self.find_overlap(frame, &frame_hashes) {
            Some((shift, score)) => {
                let append_rows = frame.height - shift;
                if self.canvas.height as u64 + append_rows as u64 > self.opts.max_height as u64 {
                    // 回退步数：调用方可能收缩后重试本轮（见 push 的超长模式）
                    self.steps -= 1;
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
                    scale: self.scale,
                    diff,
                    best_shift: diag_shift,
                    best_mean: diag_mean,
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
                    scale: self.scale,
                    diff,
                    best_shift: diag_shift,
                    best_mean: diag_mean,
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
    /// 行偏移（= 重叠行数 = 帧高 − 本轮滚动距离）。
    ///
    /// 0.1.14 起**全范围搜索 + 抗局部差异**：
    /// 1. 候选上界从"帧高−8"放开到 `min(帧高−1, 累积图高, 哈希窗口长)`。
    ///    真机上合成滚轮事件可能只滚 1~7 行（重叠 = 帧高−k），老上界直接判死；
    /// 2. 匹配判据从"整段平均"改成"**行级多数表决**"：哈希要求 ≥50% 的行几乎
    ///    完全相同、SAD 要求 ≥50% 的行平均差 < 8。这样**吸顶导航栏 / 加载中的
    ///    图 / 光标残影**这些只影响少数行的差异不会再掀翻整次匹配 ——
    ///    0.1.13 真机三次 NoMatch 的最可能成因就是 Apple 官网的吸顶栏。
    fn find_overlap(&self, frame: &RgbaBuffer, frame_hashes: &[u64]) -> Option<(u32, f32)> {
        let fh = frame.height;
        let ch = self.canvas.height;
        // 真实可行上界：重叠不可能超过"帧高−1"（整帧重复 = 没滚，已被 still 挡掉）
        let s_max = (fh - 1).min(ch).min(self.tail_hashes.len() as u32);
        let s_lo = self.opts.min_overlap.min(s_max);
        if s_max == 0 || s_lo == 0 {
            return None;
        }

        // 阶段 1a：行哈希平均 Hamming，在 [min_overlap, s_max] 取 top-3 最小
        let mut top: [(f32, u32); 3] = [(f32::MAX, 0); 3];
        for s in s_lo..=s_max {
            let mean = self.shift_mean(frame_hashes, s);
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

        // 阶段 1b：`min_overlap` 以下（滚动极小）只收"近乎完美"的候选 ——
        // 真实截图几乎不可能在几十行上哈希全中，宁少拼也不错拼。
        let mut tiny: Vec<u32> = Vec::new();
        for s in MIN_TRUST_OVERLAP..s_lo {
            if self.shift_mean(frame_hashes, s) < PERFECT_MEAN {
                tiny.push(s);
            }
        }

        // 阶段 2：行级多数表决复核全部候选
        // best = (行一致率, 这些行的平均 SAD, shift)
        let mut best: Option<(f32, f32, u32)> = None;
        let mut consider = |s: u32, best: &mut Option<(f32, f32, u32)>| {
            if s == 0 {
                return;
            }
            let (frac, sad) = region_row_vote(&self.canvas, frame, s);
            if frac >= MATCH_ROW_FRAC && sad < self.opts.match_threshold {
                match best {
                    // 行一致率高的优先，一致率相同则 SAD 小的优先
                    Some((bf, bs, _)) if *bf > frac || (*bf == frac && *bs <= sad) => {}
                    _ => *best = Some((frac, sad, s)),
                }
            }
        };
        for &(_, s) in &top {
            consider(s, &mut best);
        }
        for s in tiny {
            consider(s, &mut best);
        }
        best.map(|(frac, _, s)| (s, frac.clamp(0.0, 1.0)))
    }

    /// 在**全可行范围**（1..=min(帧高−1, 哈希窗口长)）上扫行哈希平均汉明距离，
    /// 返回 (最优 shift, 平均距离)。诊断用，也驱动 `best_shift/best_mean` 日志。
    fn scan_best_shift(&self, frame_hashes: &[u64]) -> (u32, f32) {
        let s_max = (frame_hashes.len() as u32 - 1).min(self.tail_hashes.len() as u32);
        let mut best = (f32::MAX, 0u32);
        for s in 1..=s_max {
            let mean = self.shift_mean(frame_hashes, s);
            if mean < best.0 {
                best = (mean, s);
            }
        }
        (best.1, best.0)
    }

    /// shift = s 时的行哈希平均汉明距离（0..1）。
    #[inline]
    fn shift_mean(&self, frame_hashes: &[u64], s: u32) -> f32 {
        let base = self.tail_hashes.len() - s as usize;
        let mut d = 0u32;
        for (fh_i, th_i) in frame_hashes[..s as usize]
            .iter()
            .zip(&self.tail_hashes[base..base + s as usize])
        {
            d += (fh_i ^ th_i).count_ones();
        }
        d as f32 / (s as f32 * 64.0)
    }
}

// ---------------------------------------------------------------- 纯函数

/// 放宽匹配下界时，行哈希"近乎完美"的阈值（平均汉明距离，0..1）。
const PERFECT_MEAN: f32 = 0.02;
/// 放宽匹配下界时，最少可信重叠行数（少于这个即使哈希完美也当噪声）。
const MIN_TRUST_OVERLAP: u32 = 24;
/// 行级多数表决：至少这么多比例的行"几乎完全相同"才算重叠成立。
const MATCH_ROW_FRAC: f32 = 0.5;

/// 行级多数表决复核（替代原来的整段平均 SAD）。
///
/// 返回 `(一致行比例, 这些一致行的平均 SAD)`：
/// - 比例 = 每行平均绝对差 < `ROW_SAD_OK` 的行数 / 总行数；
/// - 平均 SAD 只对"一致行"求，避免少数坏行（吸顶栏/加载图/光标）把均值抬飞。
///
/// 为什么不用整段均值：一个 44 行的吸顶导航栏在 1730 行重叠里只占 2.5%，
/// 但它的平均差可能上百，整段均值直接越过 `match_threshold` → 明明对齐了
/// 却判 NoMatch（0.1.13 真机症状）。
fn region_row_vote(canvas: &RgbaBuffer, frame: &RgbaBuffer, shift: u32) -> (f32, f32) {
    const ROW_SAD_OK: u32 = 8;
    let w = canvas.width as usize;
    let ch = canvas.height as usize;
    let s = shift as usize;
    let mut ok_rows = 0usize;
    let mut ok_sum = 0u64;
    for i in 0..s {
        let a_row = &canvas.data[(ch - s + i) * w * 4..(ch - s + i + 1) * w * 4];
        let b_row = &frame.data[i * w * 4..(i + 1) * w * 4];
        let (a_chunks, _) = a_row.as_chunks::<4>();
        let (b_chunks, _) = b_row.as_chunks::<4>();
        let mut row_sum = 0u64;
        for (pa, pb) in a_chunks.iter().zip(b_chunks.iter()) {
            row_sum += luma(pa).abs_diff(luma(pb)) as u64;
        }
        let row_mean = row_sum as u32 / w as u32;
        if row_mean < ROW_SAD_OK {
            ok_rows += 1;
            ok_sum += row_mean as u64;
        }
    }
    let frac = ok_rows as f32 / s as f32;
    let sad = if ok_rows == 0 {
        255.0
    } else {
        ok_sum as f32 / ok_rows as f32
    };
    (frac, sad)
}

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

    /// 真实重叠小于 `min_overlap`（这里 200 行 < 308）时，靠"近乎完美的行哈希"
    /// 放宽下界。
    ///
    /// 对应真机场景：一步滚太远（一帧跨过大半个视口），重叠掉到下界以下 ——
    /// 老逻辑一律 NoMatch，长截图永远只有一屏。0.1.13 真机 NoMatch 连出 5 轮、
    /// height 一动不动，是这一类问题的典型症状。
    #[test]
    fn small_overlap_below_min_still_appends() {
        let page = web_page(1400, 320);
        let opts = StitchOptions::for_frame(600); // min_overlap = 308
        let mut st = Stitcher::new(&crop_rows(&page, 0, 600), opts).unwrap();
        // 帧 = 页面 400..1000 行：与上一帧重叠 200 行（< 308），新内容 400 行
        let b = crop_rows(&page, 400, 1000);
        let step = st.push(&b).unwrap();
        assert_eq!(step.kind, StitchStepKind::Appended);
        assert_eq!(step.best_shift, 200);
        assert_eq!(step.appended, 400);
        assert_eq!(st.height(), 1000);
        assert_eq!(st.canvas(), &crop_rows(&page, 0, 1000));
    }

    /// 滚动极小（1~3 行）时重叠 = 帧高−1..帧高−3，**超过老的候选上界**
    /// （帧高−8）→ 老逻辑必然 NoMatch。0.1.13 真机三次长截图全栽在这里。
    #[test]
    fn tiny_scroll_overlap_near_frame_height_appends() {
        let page = web_page(1400, 320);
        let opts = StitchOptions::for_frame(600);
        let mut st = Stitcher::new(&crop_rows(&page, 0, 600), opts).unwrap();
        let mut y_prev = 0u32;
        for scroll in [1u32, 3] {
            let y0 = y_prev + scroll;
            let f = crop_rows(&page, y0, y0 + 600);
            let step = st.push(&f).unwrap();
            assert_eq!(step.kind, StitchStepKind::Appended, "scroll {scroll}");
            assert_eq!(step.appended, scroll, "scroll {scroll}");
            y_prev = y0;
        }
        assert_eq!(st.height(), 604); // 1 + 3 行新内容
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

    /// 超长模式：到上限不再报错，而是整体降采样一半后继续（上限随之翻倍）。
    #[test]
    fn long_mode_shrinks_instead_of_failing() {
        let opts = StitchOptions {
            max_height: 700,
            ..StitchOptions::for_frame(600)
        };
        let page = web_page(1200, 64);
        let mut st = Stitcher::new(&crop_rows(&page, 0, 600), opts).unwrap();
        // 推进 100 行：600 + 100 = 700 恰好达标
        let ok = st.push(&crop_rows(&page, 100, 700)).unwrap();
        assert_eq!(ok.kind, StitchStepKind::Appended);
        assert_eq!(ok.scale, 1);
        // 再推进 100 行会超 700 → 触发超长模式：画布减半、scale=2，然后拼上
        let step = st.push(&crop_rows(&page, 200, 800)).unwrap();
        assert_eq!(step.kind, StitchStepKind::Appended);
        assert_eq!(step.scale, 2, "超限应进入超长模式而不是报错");
        // 减半后高度 ≈ 700/2 + 50，远低于上限
        assert!(step.height < 700, "收缩后高度应明显下降: {}", step.height);
        assert_eq!(st.canvas().width, 32, "画布宽度也应同步减半");
    }

    /// 降到最大倍数后仍然超限，才真正报 TooTall（不会无限降采样）。
    #[test]
    fn long_mode_eventually_stops_at_max_scale() {
        let opts = StitchOptions {
            max_height: 1200,
            ..StitchOptions::for_frame(600)
        };
        let page = web_page(9000, 64);
        let mut st = Stitcher::new(&crop_rows(&page, 0, 600), opts).unwrap();
        let mut max_seen = 1;
        let mut err = None;
        // 一直往下推，直到报错或走完；记录见过的最大降采样倍数
        for y in (100..8400).step_by(100) {
            match st.push(&crop_rows(&page, y, y + 600)) {
                Ok(step) => max_seen = max_seen.max(step.scale),
                Err(e) => {
                    max_seen = max_seen.max(st.scale());
                    err = Some(e);
                    break;
                }
            }
        }
        assert_eq!(max_seen, 8, "应一路收缩到最大倍数 8，不会无限降采样");
        if let Some(e) = err {
            assert!(matches!(e, AlgoError::TooTall { .. }), "只有到 8 倍仍超限才报 TooTall");
        }
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
