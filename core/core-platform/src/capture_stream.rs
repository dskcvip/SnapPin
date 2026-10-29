//! Ordered, bounded frame queue for the macOS screen stream. Full-size pixels are
//! retained; sampling is used only to avoid queueing identical frames.
use snappin_core_algo::RgbaBuffer;
use std::collections::VecDeque;
#[cfg(target_os = "macos")]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

pub(crate) struct StreamFrame {
    pub image: RgbaBuffer,
    pub sequence: u64,
    pub source_time: f64,
    pub received: Instant,
}

const QUEUE_BYTES: usize = 192 * 1024 * 1024;
const QUEUE_FRAMES: usize = 120;

struct StoredFrame {
    pixels: Vec<u8>,
    sequence: u64,
    source_time: f64,
    received: Instant,
}

struct QueueState {
    frames: VecDeque<StoredFrame>,
    pending: Option<StoredFrame>,
    bytes: usize,
    sequence: u64,
    dropped: u64,
    signature: Option<u64>,
    error: Option<String>,
    closed: bool,
}

struct FrameQueue {
    width: u32,
    height: u32,
    capacity: usize,
    state: Mutex<QueueState>,
}

impl FrameQueue {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            capacity: QUEUE_FRAMES,
            state: Mutex::new(QueueState {
                frames: VecDeque::new(),
                pending: None,
                bytes: 0,
                sequence: 0,
                dropped: 0,
                signature: None,
                error: None,
                closed: false,
            }),
        }
    }

    fn push(&self, image: RgbaBuffer, source_time: f64) {
        // Compress outside the queue lock. Keep full output pixels, but retain
        // seconds of overlapping webpage frames rather than three raw frames.
        let pixels = lz4_flex::compress_prepend_size(&image.data);
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        state.sequence += 1;
        let frame = StoredFrame {
            pixels,
            source_time,
            sequence: state.sequence,
            received: Instant::now(),
        };
        if state.pending.is_none() && state.frames.len() < self.capacity
            && state.bytes + frame.pixels.len() <= QUEUE_BYTES
        {
            state.bytes += frame.pixels.len();
            state.frames.push_back(frame);
        } else {
            // Retain the earlier sequence and the latest frame. Never silently
            // overwrite the oldest overlap anchor or lose the final stopped frame.
            if state.pending.replace(frame).is_some() {
                state.dropped += 1;
            }
        }
    }

    fn pop(&self) -> Result<Option<(StreamFrame, usize, u64)>, String> {
        let stored = {
            let mut state = self.state.lock().unwrap();
            if let Some(frame) = state.frames.pop_front() {
                state.bytes -= frame.pixels.len();
                if let Some(pending) = state.pending.take() {
                    state.bytes += pending.pixels.len();
                    state.frames.push_back(pending);
                }
                Some((frame, state.frames.len(), state.dropped))
            } else if let Some(error) = state.error.take() {
                return Err(error);
            } else { None }
        };
        stored.map(|(frame, queued, dropped)| {
            let data = lz4_flex::decompress_size_prepended(&frame.pixels)
                .map_err(|e| format!("long frame decompression: {e}"))?;
            let image = RgbaBuffer::new(self.width, self.height, data).map_err(|e| e.to_string())?;
            Ok((StreamFrame { image, sequence: frame.sequence,
                source_time: frame.source_time, received: frame.received }, queued, dropped))
        }).transpose()
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.frames.clear();
        state.pending = None;
        state.bytes = 0;
    }

    #[cfg(any(target_os = "macos", test))]
    fn receive_bgra(&self, bytes: &[u8], width: usize, height: usize, stride: usize, time: f64) {
        if width != self.width as usize || height != self.height as usize {
            self.state.lock().unwrap().error = Some(format!(
                "mac stream dimensions changed: {width}x{height}, expected {}x{}",
                self.width, self.height
            ));
            return;
        }
        let mut signature = 0xcbf29ce484222325u64;
        for y in (0..height).step_by((height / 64).max(1)) {
            for x in (0..width).step_by((width / 96).max(1)) {
                let p = y * stride + x * 4;
                let color = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
                signature = (signature ^ color as u64).wrapping_mul(0x100000001b3);
            }
        }
        {
            let mut state = self.state.lock().unwrap();
            if state.closed || state.signature == Some(signature) {
                return;
            }
            state.signature = Some(signature);
        }
        let mut pixels = vec![0; width * height * 4];
        for (source, dest) in bytes
            .chunks_exact(stride)
            .take(height)
            .zip(pixels.chunks_exact_mut(width * 4))
        {
            for (bgra, rgba) in source[..width * 4]
                .chunks_exact(4)
                .zip(dest.chunks_exact_mut(4))
            {
                rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
            }
        }
        self.push(
            RgbaBuffer {
                width: width as u32,
                height: height as u32,
                data: pixels,
            },
            time,
        );
    }
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use snappin_core_algo::Rect;
    use std::ffi::{c_char, c_void, CStr};

    extern "C" {
        fn snappin_long_stream_start(
            display: u32,
            x: f64,
            y: f64,
            width: f64,
            height: f64,
            pixel_width: u32,
            pixel_height: u32,
            callback: unsafe extern "C" fn(*mut c_void, *const u8, usize, usize, usize, f64),
            context: *mut c_void,
            error: *mut c_char,
            error_size: usize,
        ) -> *mut c_void;
        fn snappin_long_stream_stop(handle: *mut c_void);
    }

    unsafe extern "C" fn on_frame(
        context: *mut c_void,
        pixels: *const u8,
        width: usize,
        height: usize,
        stride: usize,
        time: f64,
    ) {
        // Native callbacks are serialized; stop drains that queue before dropping context.
        let queue = &*(context as *const Arc<FrameQueue>);
        if pixels.is_null() {
            queue.state.lock().unwrap().error =
                Some("macOS screen stream stopped unexpectedly".into());
            return;
        }
        if width == 0 || height == 0 || stride < width.saturating_mul(4) {
            return;
        }
        let Some(length) = stride.checked_mul(height) else {
            return;
        };
        queue.receive_bgra(
            std::slice::from_raw_parts(pixels, length),
            width,
            height,
            stride,
            time,
        );
    }

    pub(crate) struct MacCaptureStream {
        handle: *mut c_void,
        context: Box<Arc<FrameQueue>>,
    }
    // Native start/stop may run on workers. The output callback accesses only
    // the synchronized queue; native stop drains callbacks before freeing it.
    unsafe impl Send for MacCaptureStream {}

    impl MacCaptureStream {
        pub fn start(display: u32, rect: Rect, width: u32, height: u32) -> Result<Self, String> {
            let mut context = Box::new(Arc::new(FrameQueue::new(width, height)));
            let mut error = [0i8; 512];
            let handle = unsafe {
                snappin_long_stream_start(
                    display,
                    rect.x as f64,
                    rect.y as f64,
                    rect.w as f64,
                    rect.h as f64,
                    width,
                    height,
                    on_frame,
                    (&mut *context as *mut Arc<FrameQueue>).cast(),
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            if handle.is_null() {
                return Err(unsafe { CStr::from_ptr(error.as_ptr()) }
                    .to_string_lossy()
                    .into_owned());
            }
            eprintln!(
                "[snappin] mac long stream started: {}x{} 30fps lossless queue={} frames (+ latest)",
                width, height, context.capacity
            );
            Ok(Self { handle, context })
        }

        pub fn next(&self) -> Result<Option<(StreamFrame, usize, u64)>, String> {
            self.context.pop()
        }

        pub fn stop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    snappin_long_stream_stop(self.handle);
                }
                self.handle = std::ptr::null_mut();
                eprintln!("[snappin] mac long stream stopped");
            }
        }
    }

    impl Drop for MacCaptureStream {
        fn drop(&mut self) {
            self.stop();
            self.context.close();
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) use native::MacCaptureStream;

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(value: u8) -> RgbaBuffer {
        RgbaBuffer::new(2, 2, vec![value; 16]).unwrap()
    }

    #[test]
    fn slow_consumer_preserves_earlier_frames_and_final_frame() {
        let mut queue = FrameQueue::new(2, 2);
        queue.capacity = 2;
        for value in 1..=5 {
            queue.push(frame(value), value as f64);
        }
        let mut values = Vec::new();
        while let Some((frame, _, dropped)) = queue.pop().unwrap() {
            values.push(frame.image.data[0]);
            assert_eq!(dropped, 2);
        }
        assert_eq!(values, [1, 2, 5]);
    }

    #[test]
    fn producer_and_consumer_keep_frame_order() {
        let mut queue = FrameQueue::new(2, 2);
        queue.capacity = 2;
        for value in 1..=3 {
            queue.push(frame(value), value as f64);
        }
        assert_eq!(queue.pop().unwrap().unwrap().0.sequence, 1);
        queue.push(frame(4), 4.0);
        for sequence in 2..=4 {
            let (frame, _, dropped) = queue.pop().unwrap().unwrap();
            assert_eq!(frame.sequence, sequence);
            assert_eq!(frame.source_time, sequence as f64);
            assert!(frame.received <= Instant::now());
            assert_eq!(dropped, 0);
        }
    }

    #[test]
    fn cancellation_discards_queued_frames_and_rejects_late_callbacks() {
        let queue = FrameQueue::new(2, 2);
        queue.push(frame(1), 1.0);
        queue.close();
        queue.push(frame(2), 2.0);
        assert!(queue.pop().unwrap().is_none());
    }

    #[test]
    fn retina_queue_has_a_memory_budget() {
        let queue = FrameQueue::new(3344, 1676);
        assert_eq!(queue.capacity, 120);
        assert_eq!(queue.state.lock().unwrap().bytes, 0);
    }

    #[test]
    fn retina_scroll_burst_survives_a_stalled_stitcher_without_missing_pixels() {
        use snappin_core_algo::stitch::{Stitcher, StitchOptions, StitchStepKind};
        let (width, height, frame_h) = (3344u32, 6500u32, 1676u32);
        let mut data = vec![0; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let n = y.wrapping_mul(2654435761) ^ (x / 16).wrapping_mul(2246822519);
                let p = ((y * width + x) * 4) as usize;
                data[p..p + 4].copy_from_slice(&[(n >> 8) as u8, (n >> 16) as u8, (n >> 24) as u8, 255]);
            }
        }
        let crop = |y: u32| RgbaBuffer::new(width, frame_h,
            data[(y * width * 4) as usize..((y + frame_h) * width * 4) as usize].to_vec()).unwrap();
        let queue = FrameQueue::new(width, frame_h);
        // Model a consumer pause: all frames arrive before any matching runs.
        // The old three-frame queue dropped almost this entire burst.
        for i in 0..=30 { queue.push(crop(i * 150), i as f64 / 30.0); }
        {
            let state = queue.state.lock().unwrap();
            assert_eq!(state.dropped, 0);
            assert!(state.bytes <= QUEUE_BYTES);
            eprintln!("31 Retina frames retained losslessly in {} bytes", state.bytes);
        }
        let mut st = Stitcher::new(&crop(0), StitchOptions::for_frame(frame_h)).unwrap();
        let mut expected_sequence = 1;
        while let Some((frame, _, dropped)) = queue.pop().unwrap() {
            assert_eq!(frame.sequence, expected_sequence);
            expected_sequence += 1;
            assert_eq!(dropped, 0);
            assert_ne!(st.push(&frame.image).unwrap().kind, StitchStepKind::NoMatch);
        }
        assert_eq!(st.height(), 4500 + frame_h);
        assert_eq!(st.canvas().data, data[..((4500 + frame_h) * width * 4) as usize]);
    }

    #[test]
    fn native_bgra_rows_keep_full_resolution_and_skip_identical_frames() {
        let queue = FrameQueue::new(2, 2);
        // Each native row contains two pixels and four padding bytes.
        let bytes = [
            3, 2, 1, 0, 6, 5, 4, 0, 99, 99, 99, 99, 9, 8, 7, 0, 12, 11, 10, 0, 99, 99, 99, 99,
        ];
        queue.receive_bgra(&bytes, 2, 2, 12, 1.0);
        queue.receive_bgra(&bytes, 2, 2, 12, 2.0);
        let (frame, _, _) = queue.pop().unwrap().unwrap();
        assert_eq!((frame.image.width, frame.image.height), (2, 2));
        assert_eq!(
            frame.image.data,
            [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
        );
        assert!(queue.pop().unwrap().is_none());
    }
}
