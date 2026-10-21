//! SnapPin 纯算法层（core-algo）。
//!
//! # 设计约束（远期 C++ 移植预留，勿破坏）
//! 1. 只依赖 `std`，零第三方依赖；
//! 2. 只处理 RGBA8 字节缓冲（每像素 4 字节，行优先，左上原点）；
//! 3. 全部为纯函数 / 值类型：无全局状态、无 IO、无线程、无平台 API；
//! 4. 每个公开 API 都有单元测试。
//!
//! 这些约束使本 crate 成为一份"移植说明书"：将来移植到 C++（鸿蒙 PC 路线）
//! 时是机械翻译，而不是重新设计。

use std::fmt;

pub mod draw;
pub mod ocr;
pub mod shapes;
pub mod stitch;

/// 算法层错误。零依赖，因此手写 `Display` / `Error`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlgoError {
    /// 数据长度与 `width * height * 4` 不匹配
    BufferLengthMismatch { expected: usize, actual: usize },
    /// 宽或高为 0
    ZeroSize,
    /// 选区完全位于图像之外
    RectOutOfBounds,
    /// 帧高度 < 2，无法拼接
    FrameTooShort { h: u32 },
    /// 新帧宽度与累积图不一致（长截图要求等宽）
    WidthMismatch { expected: u32, actual: u32 },
    /// 累积图超过高度上限
    TooTall { max: u32 },
}

impl fmt::Display for AlgoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AlgoError::BufferLengthMismatch { expected, actual } => write!(
                f,
                "rgba buffer length mismatch: expected {}, got {}",
                expected, actual
            ),
            AlgoError::ZeroSize => write!(f, "image width/height must be non-zero"),
            AlgoError::RectOutOfBounds => write!(f, "selection rect is fully outside the image"),
            AlgoError::FrameTooShort { h } => {
                write!(f, "frame height {} is too short to stitch", h)
            }
            AlgoError::WidthMismatch { expected, actual } => write!(
                f,
                "frame width {} does not match canvas width {}",
                actual, expected
            ),
            AlgoError::TooTall { max } => write!(f, "stitched image exceeds max height {}", max),
        }
    }
}

impl std::error::Error for AlgoError {}

/// 矩形选区。逻辑像素，左上原点，`w`/`h` 非负。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    /// 由拖拽两点构造，自动归一化（支持从右往左、从下往上拖拽）。
    pub fn from_drag(x1: i32, y1: i32, x2: i32, y2: i32) -> Self {
        Self {
            x: x1.min(x2),
            y: y1.min(y2),
            w: (x2 - x1).unsigned_abs(),
            h: (y2 - y1).unsigned_abs(),
        }
    }

    pub fn area(&self) -> u64 {
        self.w as u64 * self.h as u64
    }

    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// 与图像边界求交；完全在界外返回 `None`。
    pub fn clamp_to(&self, width: u32, height: u32) -> Option<Rect> {
        if width == 0 || height == 0 || self.is_empty() {
            return None;
        }
        let x0 = self.x.max(0) as i64;
        let y0 = self.y.max(0) as i64;
        let x1 = (self.x as i64 + self.w as i64).min(width as i64);
        let y1 = (self.y as i64 + self.h as i64).min(height as i64);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Rect {
            x: x0 as i32,
            y: y0 as i32,
            w: (x1 - x0) as u32,
            h: (y1 - y0) as u32,
        })
    }
}

/// RGBA8 图像缓冲。`data.len() == width * height * 4`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaBuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl RgbaBuffer {
    /// 构造并校验长度。
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self, AlgoError> {
        if width == 0 || height == 0 {
            return Err(AlgoError::ZeroSize);
        }
        let expected = width as usize * height as usize * 4;
        if data.len() != expected {
            return Err(AlgoError::BufferLengthMismatch {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// 裁剪出子区域（自动与图像边界求交，完全界外报错）。
    pub fn crop(&self, rect: &Rect) -> Result<RgbaBuffer, AlgoError> {
        let r = rect.clamp_to(self.width, self.height).ok_or(AlgoError::RectOutOfBounds)?;
        let row_bytes = self.width as usize * 4;
        let mut out = Vec::with_capacity(r.area() as usize * 4);
        for row in 0..r.h as usize {
            let start = (r.y as usize + row) * row_bytes + r.x as usize * 4;
            out.extend_from_slice(&self.data[start..start + r.w as usize * 4]);
        }
        RgbaBuffer::new(r.w, r.h, out)
    }

    /// 读取像素；越界返回 `None`。
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = (y as usize * self.width as usize + x as usize) * 4;
        Some([
            self.data[i],
            self.data[i + 1],
            self.data[i + 2],
            self.data[i + 3],
        ])
    }

    /// 原地水平翻转。
    pub fn flip_horizontal_in_place(&mut self) {
        let row_bytes = self.width as usize * 4;
        for row in self.data.chunks_exact_mut(row_bytes) {
            for x in 0..(self.width as usize / 2) {
                let l = x * 4;
                let r = (self.width as usize - 1 - x) * 4;
                for c in 0..4 {
                    row.swap(l + c, r + c);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造 width x height 的测试图，像素值 = (x, y, 0, 255)
    fn test_image(width: u32, height: u32) -> RgbaBuffer {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                data.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        RgbaBuffer::new(width, height, data).unwrap()
    }

    #[test]
    fn from_drag_normalizes_all_directions() {
        assert_eq!(
            Rect::from_drag(10, 10, 30, 40),
            Rect { x: 10, y: 10, w: 20, h: 30 }
        );
        // 从右往左、从下往上
        assert_eq!(
            Rect::from_drag(30, 40, 10, 10),
            Rect { x: 10, y: 10, w: 20, h: 30 }
        );
    }

    #[test]
    fn clamp_to_intersects_bounds() {
        let r = Rect { x: -5, y: -5, w: 20, h: 20 };
        assert_eq!(
            r.clamp_to(10, 10),
            Some(Rect { x: 0, y: 0, w: 10, h: 10 })
        );
        // 完全在界外
        let outside = Rect { x: 100, y: 100, w: 10, h: 10 };
        assert_eq!(outside.clamp_to(10, 10), None);
        // 部分相交
        let partial = Rect { x: 5, y: 5, w: 20, h: 20 };
        assert_eq!(
            partial.clamp_to(10, 10),
            Some(Rect { x: 5, y: 5, w: 5, h: 5 })
        );
    }

    #[test]
    fn crop_returns_exact_pixels() {
        let img = test_image(4, 4);
        let cropped = img
            .crop(&Rect { x: 1, y: 1, w: 2, h: 2 })
            .expect("crop should succeed");
        assert_eq!(cropped.width, 2);
        assert_eq!(cropped.height, 2);
        // 第一行：(1,1) 与 (2,1)
        assert_eq!(&cropped.data[0..4], &[1, 1, 0, 255]);
        assert_eq!(&cropped.data[4..8], &[2, 1, 0, 255]);
        // 第二行：(1,2) 与 (2,2)
        assert_eq!(&cropped.data[8..12], &[1, 2, 0, 255]);
        assert_eq!(&cropped.data[12..16], &[2, 2, 0, 255]);
    }

    #[test]
    fn crop_clamps_to_bounds() {
        let img = test_image(4, 4);
        let cropped = img
            .crop(&Rect { x: 2, y: 2, w: 10, h: 10 })
            .expect("crop should clamp");
        assert_eq!((cropped.width, cropped.height), (2, 2));
        assert_eq!(cropped.pixel(0, 0), Some([2, 2, 0, 255]));
    }

    #[test]
    fn crop_outside_errors() {
        let img = test_image(4, 4);
        let err = img
            .crop(&Rect { x: 10, y: 10, w: 5, h: 5 })
            .unwrap_err();
        assert_eq!(err, AlgoError::RectOutOfBounds);
    }

    #[test]
    fn new_validates_length() {
        let err = RgbaBuffer::new(2, 2, vec![0; 8]).unwrap_err();
        assert_eq!(
            err,
            AlgoError::BufferLengthMismatch {
                expected: 16,
                actual: 8
            }
        );
        assert_eq!(RgbaBuffer::new(0, 2, vec![]).unwrap_err(), AlgoError::ZeroSize);
    }

    #[test]
    fn flip_horizontal_mirrors_pixels() {
        let mut img = test_image(2, 1);
        img.flip_horizontal_in_place();
        assert_eq!(img.pixel(0, 0), Some([1, 0, 0, 255]));
        assert_eq!(img.pixel(1, 0), Some([0, 0, 0, 255]));
    }
}
