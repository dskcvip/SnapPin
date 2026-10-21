//! OCR 文字识别（core-platform）：PP-OCRv4 mobile + rten 纯 Rust 推理。
//!
//! # 为什么是这套组合
//!
//! - **rten**：纯 Rust ONNX 运行时，静态链接进产物，不需要像 `ort` 那样
//!   随应用分发 15MB 的 `onnxruntime.dll`，也没有 build 时下载原生二进制的
//!   环节（github.com 在本机时通时断，那条路不可靠）。
//! - **PP-OCRv4 mobile**：检测 4.7MB + 识别 10.8MB，中文场景精度/体积平衡最好，
//!   且 RapidOCR 在 ModelScope 上提供官方 ONNX 导出（SHA256 已核对）。
//! - **模型不进安装包**：按需下载到固定目录，仓库侧 `models/` 托管供用户下载，
//!   见 `scripts/fetch_models.py` 与 `models/README.md`。缺模型时 OCR 不可用，
//!   但应用其余功能完全正常。
//!
//! # 管线
//!
//! ```text
//! 原图 ─┬─ letterbox 归一化 ─→ det 模型 ─→ 概率图 ─→ 二值化 + 连通域 ─→ 文本框
//!       └─ 逐框裁剪 ─→ 归一化到高 48 ─→ rec 模型 ─→ CTC 序列 ─→ 字典解码 ─→ 文字
//! ```
//!
//! 概率图之后的后处理与 CTC 解码在 `snappin_core_algo::ocr`（纯函数、可移植），
//! 本模块只做「胶水」：张量构造、模型推理、坐标换算。

use anyhow::{Context, Result, anyhow, bail};
use image::{DynamicImage, RgbaImage, imageops::FilterType};
use rten::Model;
use rten_tensor::{AsView, Layout, Tensor};
use snappin_core_algo::ocr::{self, DetBox, DetOptions};
use snappin_core_algo::{RgbaBuffer, Rect};
use std::path::Path;

/// 模型目录下三个文件的文件名（与 `scripts/fetch_models.py` 的落盘名一致）。
pub const DET_FILE: &str = "det.onnx";
pub const REC_FILE: &str = "rec.onnx";
pub const DICT_FILE: &str = "ppocr_keys_v1.txt";

/// 检测端输入的最长边（像素）。PP-OCRv4 mobile 官方推荐 736。
pub const DET_LIMIT_SIDE: u32 = 736;
/// 检测端输入尺寸对齐倍数（官方预处理 `resize_img_type0` 向上取整到 32）。
pub const DET_MULTIPLE: u32 = 32;
/// 识别端归一化高度（PP-OCR 全系固定 48）。
pub const REC_HEIGHT: u32 = 48;
/// 识别端输入宽度上限（PP-OCR 官方 320）。
pub const REC_MAX_WIDTH: u32 = 320;
/// rec 模型输出类别数：blank(1) + 字典(6623) + space(1)。
/// 字典文件行数变化时同步调整（`OcrEngine::load` 会校验）。
pub const REC_NUM_CLASSES: usize = 6625;

/// PP-OCR 模型是否期望 **BGR** 通道序。
///
/// PaddleOCR 全链路用 `cv2.imread`（BGR）训练与导出，ONNX 模型的 conv 权重
/// 是按 BGR 学的；而我们的截图缓冲是 RGBA（R 在前）。若不交换 R/B，
/// 彩色文字会识别错乱（灰度文字上两者无差别，实测无法区分，故按官方
/// 管线的 cv2 约定取 BGR）。
const MODEL_EXPECTS_BGR: bool = true;

/// 一行识别结果。
#[derive(Debug, Clone)]
pub struct OcrLine {
    pub text: String,
    /// 识别置信度（CTC 各时刻最大概率的均值，0..1）
    pub score: f32,
    /// 检测框：**原图像素坐标**，`max_*` 为含端
    pub box_: DetBox,
}

/// OCR 引擎（两个模型 + 字典）。加载一次，可反复 `recognize`。
pub struct OcrEngine {
    det: Model,
    rec: Model,
    dict: Vec<String>,
}

impl OcrEngine {
    /// 从模型目录加载（三个文件齐全才能成功）。
    pub fn load(dir: &Path) -> Result<Self> {
        let det = Model::load_file(dir.join(DET_FILE)).context("load det model failed")?;
        let rec = Model::load_file(dir.join(REC_FILE)).context("load rec model failed")?;
        let raw = std::fs::read_to_string(dir.join(DICT_FILE)).context("read dict failed")?;
        // 字典按行解析；忽略末尾空行（文件可能以换行结尾）
        let dict: Vec<String> = raw
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        if dict.len() + 2 != REC_NUM_CLASSES {
            bail!(
                "dict has {} chars, but rec model expects {} classes (blank + dict + space); \
                 please update models (see models/README.md)",
                dict.len(),
                REC_NUM_CLASSES
            );
        }
        Ok(Self { det, rec, dict })
    }

    /// 识别整张图里的文字行（阅读顺序）。
    pub fn recognize(&self, img: &RgbaBuffer) -> Result<Vec<OcrLine>> {
        self.recognize_with_order(img, MODEL_EXPECTS_BGR)
    }

    /// [`Self::recognize`] 的通道序可变版（仅测试用于实测 BGR/RGB 差异）。
    fn recognize_with_order(&self, img: &RgbaBuffer, bgr: bool) -> Result<Vec<OcrLine>> {
        let boxes = self.detect(img, bgr)?;
        let mut lines = Vec::with_capacity(boxes.len());
        for b in &boxes {
            if let Some(line) = self.recognize_box(img, b, bgr)? {
                lines.push(line);
            }
        }
        Ok(lines)
    }

    /// 检测阶段：letterbox → det 推理 → 后处理 → 框映射回原图坐标。
    fn detect(&self, img: &RgbaBuffer, bgr: bool) -> Result<Vec<DetBox>> {
        let (nw, nh) = ocr::det_input_size(img.width, img.height, DET_LIMIT_SIDE, DET_MULTIPLE);
        let dyn_img = to_dynamic_image(img);
        let resized = dyn_img.resize_exact(nw, nh, FilterType::Triangle);
        let input = pack_nchw(&resized, bgr);
        let tensor = Tensor::<f32>::from_data(&[1, 3, nh as usize, nw as usize], input);
        let out = self
            .det
            .run_one((&tensor).into(), None)
            .context("det inference failed")?;
        let t = out
            .into_tensor::<f32>()
            .ok_or_else(|| anyhow!("det output is not an f32 tensor"))?;
        let probs = t
            .data()
            .ok_or_else(|| anyhow!("det output is not contiguous"))?;
        let shape = t.shape();
        if shape.len() != 4 || shape[0] != 1 || shape[1] != 1 {
            bail!("unexpected det output shape {:?}", shape);
        }
        let (ow, oh) = (shape[3] as usize, shape[2] as usize);
        if probs.len() != ow * oh {
            bail!(
                "det output len {} != shape {}x{}",
                probs.len(),
                ow,
                oh
            );
        }
        let boxes = ocr::detect_text_boxes(probs, ow, oh, &DetOptions::default());
        // 检测空间 → 原图坐标（两个方向分别除，letterbox 的 32 取整会让
        // 两个方向的缩放比略有差异，不能共用一个 ratio）
        let rx = img.width as f32 / nw as f32;
        let ry = img.height as f32 / nh as f32;
        Ok(boxes
            .iter()
            .map(|b| DetBox {
                min_x: (b.min_x * rx).clamp(0.0, (img.width.saturating_sub(1)) as f32),
                min_y: (b.min_y * ry).clamp(0.0, (img.height.saturating_sub(1)) as f32),
                max_x: (b.max_x * rx).clamp(0.0, (img.width.saturating_sub(1)) as f32),
                max_y: (b.max_y * ry).clamp(0.0, (img.height.saturating_sub(1)) as f32),
                score: b.score,
            })
            .collect())
    }

    /// 识别阶段：逐框裁剪 → 归一化到高 48 → rec 推理 → CTC 解码。
    fn recognize_box(&self, img: &RgbaBuffer, b: &DetBox, bgr: bool) -> Result<Option<OcrLine>> {
        // 框（含端、f32）→ 裁剪矩形（半开区间），夹到图内
        let x0 = b.min_x.floor().max(0.0) as u32;
        let y0 = b.min_y.floor().max(0.0) as u32;
        let x1 = ((b.max_x.floor() as u32) + 1).min(img.width);
        let y1 = ((b.max_y.floor() as u32) + 1).min(img.height);
        if x1 <= x0 || y1 <= y0 {
            return Ok(None);
        }
        let crop = img.crop(&Rect {
            x: x0 as i32,
            y: y0 as i32,
            w: x1 - x0,
            h: y1 - y0,
        })?;
        let rw = ocr::rec_input_size(crop.width, crop.height, REC_HEIGHT, REC_MAX_WIDTH);
        let dyn_img = to_dynamic_image(&crop);
        let resized = dyn_img.resize_exact(rw, REC_HEIGHT, FilterType::Triangle);
        // NCHW + 右侧补零到 320（PaddleOCR 在归一化之后补 0，即补「中灰」）
        let plane = (REC_HEIGHT * REC_MAX_WIDTH) as usize;
        let mut input = vec![0f32; 3 * plane];
        let rgba = resized.to_rgba8();
        for y in 0..REC_HEIGHT as usize {
            for x in 0..rw as usize {
                let p = rgba.get_pixel(x as u32, y as u32);
                let (c0, c1, c2) = order_channels(p[0], p[1], p[2], bgr);
                let i = y * REC_MAX_WIDTH as usize + x;
                input[i] = normalize(c0);
                input[plane + i] = normalize(c1);
                input[2 * plane + i] = normalize(c2);
            }
        }
        let tensor = Tensor::<f32>::from_data(
            &[1, 3, REC_HEIGHT as usize, REC_MAX_WIDTH as usize],
            input,
        );
        let out = self
            .rec
            .run_one((&tensor).into(), None)
            .context("rec inference failed")?;
        let t = out
            .into_tensor::<f32>()
            .ok_or_else(|| anyhow!("rec output is not an f32 tensor"))?;
        let probs = t
            .data()
            .ok_or_else(|| anyhow!("rec output is not contiguous"))?;
        let shape = t.shape();
        if shape.len() != 3 || shape[0] != 1 {
            bail!("unexpected rec output shape {:?}", shape);
        }
        let (steps, classes) = (shape[1] as usize, shape[2] as usize);
        if probs.len() != steps * classes {
            bail!(
                "rec output len {} != shape {}x{}",
                probs.len(),
                steps,
                classes
            );
        }
        let (text, score) = ocr::ctc_decode(probs, steps, classes, &self.dict);
        if text.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(OcrLine {
            text,
            score,
            box_: *b,
        }))
    }
}

/// 模型目录是否齐全（三个文件都存在且非空）。
///
/// 前端据此提示「去下载模型」而不是弹推理错误。
pub fn models_present(dir: &Path) -> bool {
    [DET_FILE, REC_FILE, DICT_FILE].iter().all(|name| {
        dir.join(name)
            .metadata()
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false)
    })
}

/// 通道序：PP-OCR 模型按 BGR 训练（cv2 读图），我们的缓冲是 RGBA。
fn order_channels(r: u8, g: u8, b: u8, bgr: bool) -> (u8, u8, u8) {
    if bgr {
        (b, g, r)
    } else {
        (r, g, b)
    }
}

/// PP-OCR 归一化：`(x/255 - 0.5) / 0.5`，即映射到 [-1, 1]。
fn normalize(v: u8) -> f32 {
    (v as f32 / 255.0 - 0.5) / 0.5
}

fn to_dynamic_image(buf: &RgbaBuffer) -> DynamicImage {
    let image =
        RgbaImage::from_raw(buf.width, buf.height, buf.data.clone()).expect("valid rgba buffer");
    DynamicImage::ImageRgba8(image)
}

/// 打包成 NCHW f32（`[3, h, w]`，行主序），带 PP-OCR 归一化。
fn pack_nchw(img: &DynamicImage, bgr: bool) -> Vec<f32> {
    let rgba = img.to_rgba8();
    let w = rgba.width() as usize;
    let h = rgba.height() as usize;
    let plane = w * h;
    let mut out = vec![0f32; 3 * plane];
    for (i, p) in rgba.pixels().enumerate() {
        let (c0, c1, c2) = order_channels(p[0], p[1], p[2], bgr);
        out[i] = normalize(c0);
        out[plane + i] = normalize(c1);
        out[2 * plane + i] = normalize(c2);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn models_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("models/ocr")
    }

    /// 摸清两个模型的输出张量形状（一次性探针，不参与常规回归）：
    /// `cargo test -p snappin-core-platform probe_model_io -- --ignored --nocapture`
    #[test]
    #[ignore = "需要 models/ 下的模型文件，仅用于开发期摸底"]
    fn probe_model_io() {
        let dir = models_dir();
        // det：喂一个 [1,3,736,736] 的零图，看概率图形状与数值范围
        let det = rten::Model::load_file(dir.join(DET_FILE)).expect("load det");
        let det_in: Tensor<f32> = Tensor::from_data(
            &[1, 3, DET_LIMIT_SIDE as usize, DET_LIMIT_SIDE as usize],
            vec![0.0f32; 3 * DET_LIMIT_SIDE as usize * DET_LIMIT_SIDE as usize],
        );
        let det_out = det.run_one((&det_in).into(), None).expect("run det");
        let t = det_out.into_tensor::<f32>().expect("f32 tensor");
        let d = t.data().unwrap_or(&[]);
        let mn = d.iter().cloned().fold(f32::INFINITY, f32::min);
        let mx = d.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        println!("det out shape={:?} len={} min={} max={}", t.shape(), t.len(), mn, mx);

        // rec：喂一个 [1,3,48,320] 的零图，看 CTC 概率图形状
        let rec = rten::Model::load_file(dir.join(REC_FILE)).expect("load rec");
        let rec_in: Tensor<f32> = Tensor::from_data(
            &[1, 3, REC_HEIGHT as usize, REC_MAX_WIDTH as usize],
            vec![0.0f32; 3 * REC_HEIGHT as usize * REC_MAX_WIDTH as usize],
        );
        let rec_out = rec.run_one((&rec_in).into(), None).expect("run rec");
        let t = rec_out.into_tensor::<f32>().expect("f32 tensor");
        let d = t.data().unwrap_or(&[]);
        let mn = d.iter().cloned().fold(f32::INFINITY, f32::min);
        let mx = d.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        println!("rec out shape={:?} len={} min={} max={}", t.shape(), t.len(), mn, mx);
    }

    /// 分阶段基准：模型加载 / det 推理 / 后处理 / rec 推理各耗时多少。
    ///
    /// `cargo test -p snappin-core-platform bench_phases -- --ignored --nocapture`
    #[test]
    #[ignore = "开发期性能摸底"]
    fn bench_phases() {
        println!(
            "cpus: {}",
            std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0)
        );
        let dir = models_dir();
        let t = std::time::Instant::now();
        let engine = OcrEngine::load(&dir).expect("load engine");
        println!("load engine: {}ms", t.elapsed().as_millis());

        for (w, h) in [(736u32, 736u32), (1312, 736), (1920, 1088)] {
            let input = vec![0.5f32; 3 * w as usize * h as usize];
            let tensor = Tensor::<f32>::from_data(&[1, 3, h as usize, w as usize], input);
            let t = std::time::Instant::now();
            let out = engine.det.run_one((&tensor).into(), None).unwrap();
            println!("det {}x{}: {}ms", w, h, t.elapsed().as_millis());
            std::hint::black_box(&out);
        }

        // 真图：截主屏中心 1/3，分离 det 推理与后处理的耗时
        let monitors = xcap::Monitor::all().expect("enumerate monitors");
        let primary = monitors
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .expect("no primary monitor");
        let rgba = primary.capture_image().expect("capture failed");
        let img = RgbaBuffer::new(rgba.width(), rgba.height(), rgba.into_raw()).unwrap();
        let region = Rect {
            x: (img.width / 3) as i32,
            y: (img.height / 3) as i32,
            w: img.width / 3,
            h: img.height / 3,
        };
        let img = img.crop(&region).unwrap();
        println!("real region: {}x{}", img.width, img.height);
        let (nw, nh) = ocr::det_input_size(img.width, img.height, DET_LIMIT_SIDE, DET_MULTIPLE);
        let dyn_img = to_dynamic_image(&img);
        let resized = dyn_img.resize_exact(nw, nh, FilterType::Triangle);
        let t = std::time::Instant::now();
        let input = pack_nchw(&resized, MODEL_EXPECTS_BGR);
        println!(
            "  prepack {}x{} (input {}x{}): {}ms",
            img.width,
            img.height,
            nw,
            nh,
            t.elapsed().as_millis()
        );
        let tensor = Tensor::<f32>::from_data(&[1, 3, nh as usize, nw as usize], input);
        let t = std::time::Instant::now();
        let out = engine.det.run_one((&tensor).into(), None).unwrap();
        println!("  det inference: {}ms", t.elapsed().as_millis());
        let t = std::time::Instant::now();
        let tensor_out = out.into_tensor::<f32>().unwrap();
        let probs = tensor_out.data().unwrap();
        let boxes = ocr::detect_text_boxes(
            probs,
            nw as usize,
            nh as usize,
            &DetOptions::default(),
        );
        println!(
            "  postprocess: {}ms → {} boxes",
            t.elapsed().as_millis(),
            boxes.len()
        );
        let t = std::time::Instant::now();
        for b in &boxes {
            let _ = engine.recognize_box(&img, b, MODEL_EXPECTS_BGR).unwrap();
        }
        println!("  rec x{}: {}ms", boxes.len(), t.elapsed().as_millis());
    }

    /// 真机全链路验证：截主屏 → 全管线 OCR → 打印结果。
    ///
    /// 需要 models/ 与真实显示器，开发期手动跑：
    /// `cargo test -p snappin-core-platform ocr_real_screen -- --ignored --nocapture`
    ///
    /// 它同时回答两个只能实测的问题：
    /// 1. 模型期望 BGR 还是 RGB（两种通道序各跑一遍，输出可读性立见分晓）；
    /// 2. 检测后处理参数（阈值/unclip）在真实截图上的表现。
    #[test]
    #[ignore = "需要真实显示器与 models/ 下的模型文件，开发期手动跑"]
    fn ocr_real_screen() {
        let dir = models_dir();
        assert!(models_present(&dir), "models not present in {:?}", dir);
        let monitors = xcap::Monitor::all().expect("enumerate monitors");
        let primary = monitors
            .into_iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .expect("no primary monitor");
        let rgba = primary.capture_image().expect("capture failed");
        let img = RgbaBuffer::new(rgba.width(), rgba.height(), rgba.into_raw())
            .expect("valid capture buffer");
        println!("captured {}x{}", img.width, img.height);
        // 控制时长：只取中间一块（足够覆盖典型文本）
        let region = Rect {
            x: (img.width / 4) as i32,
            y: (img.height / 4) as i32,
            w: img.width / 2,
            h: img.height / 2,
        };
        let img = img.crop(&region).expect("crop failed");
        println!("testing region {}x{}", img.width, img.height);

        let engine = OcrEngine::load(&dir).expect("load engine");
        for bgr in [true, false] {
            let t0 = std::time::Instant::now();
            let lines = engine
                .recognize_with_order(&img, bgr)
                .expect("recognize failed");
            let ms = t0.elapsed().as_millis();
            println!(
                "===== bgr={} → {} lines in {}ms =====",
                bgr,
                lines.len(),
                ms
            );
            for l in &lines {
                println!(
                    "  [{:.2}] ({:.0},{:.0})-({:.0},{:.0}) {}",
                    l.score,
                    l.box_.min_x,
                    l.box_.min_y,
                    l.box_.max_x,
                    l.box_.max_y,
                    l.text
                );
            }
        }
    }
}
