//! Ordered, bounded frame queue for the macOS screen stream. Full-size pixels are
//! retained; sampling is used only to avoid queueing identical frames.
use snappin_core_algo::RgbaBuffer;
use std::collections::VecDeque;
use std::sync::Mutex;
#[cfg(any(target_os = "macos", test))]
use std::sync::{Arc, Condvar};
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

    #[cfg(test)]
    fn push(&self, image: RgbaBuffer, source_time: f64) {
        self.push_received(image, source_time, Instant::now());
    }

    fn push_received(&self, image: RgbaBuffer, source_time: f64, received: Instant) {
        // Compress outside the queue lock. Keep full output pixels, but retain
        // seconds of overlapping webpage frames rather than three raw frames.
        let started = Instant::now();
        let pixels = lz4_flex::compress_prepend_size(&image.data);
        if started.elapsed().as_millis() > 50 {
            eprintln!(
                "[perf] long compress elapsed={}ms raw={} packed={}",
                started.elapsed().as_millis(),
                image.data.len(),
                pixels.len()
            );
        }
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        state.sequence += 1;
        let frame = StoredFrame {
            pixels,
            source_time,
            sequence: state.sequence,
            received,
        };
        if state.pending.is_none()
            && state.frames.len() < self.capacity
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
            } else if let Some(frame) = state.pending.take() {
                Some((frame, 0, state.dropped))
            } else if let Some(error) = state.error.take() {
                return Err(error);
            } else {
                None
            }
        };
        stored
            .map(|(frame, queued, dropped)| {
                let data = lz4_flex::decompress_size_prepended(&frame.pixels)
                    .map_err(|e| format!("long frame decompression: {e}"))?;
                let image =
                    RgbaBuffer::new(self.width, self.height, data).map_err(|e| e.to_string())?;
                Ok((
                    StreamFrame {
                        image,
                        sequence: frame.sequence,
                        source_time: frame.source_time,
                        received: frame.received,
                    },
                    queued,
                    dropped,
                ))
            })
            .transpose()
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.frames.clear();
        state.pending = None;
        state.bytes = 0;
    }

    #[cfg(test)]
    fn receive_bgra(&self, bytes: &[u8], width: usize, height: usize, stride: usize, time: f64) {
        self.receive_bgra_at(bytes, width, height, stride, time, Instant::now());
    }

    #[cfg(any(target_os = "macos", test))]
    fn receive_bgra_at(
        &self,
        bytes: &[u8],
        width: usize,
        height: usize,
        stride: usize,
        time: f64,
        received: Instant,
    ) {
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
        self.push_received(
            RgbaBuffer {
                width: width as u32,
                height: height as u32,
                data: pixels,
            },
            time,
            received,
        );
    }
}

// Native capture must release its IOSurface immediately. Hashing, BGRA
// conversion and compression run on this separate worker, never on the SCK
// sample-handler queue. Both raw and compressed backlogs are bounded.
#[cfg(any(target_os = "macos", test))]
struct RawFrame {
    bytes: Vec<u8>,
    width: usize,
    height: usize,
    stride: usize,
    time: f64,
    received: Instant,
}
#[cfg(any(target_os = "macos", test))]
#[derive(Default)]
struct RawState {
    frames: VecDeque<RawFrame>,
    pending: Option<RawFrame>,
    bytes: usize,
    closed: bool,
    dropped: u64,
    previous_time: Option<f64>,
}
#[cfg(any(target_os = "macos", test))]
struct FrameIngress {
    output: Arc<FrameQueue>,
    state: Mutex<RawState>,
    ready: Condvar,
}
#[cfg(any(target_os = "macos", test))]
impl FrameIngress {
    const RAW_BUDGET: usize = 128 * 1024 * 1024;

    fn new(output: Arc<FrameQueue>) -> Self {
        Self {
            output,
            state: Mutex::new(RawState::default()),
            ready: Condvar::new(),
        }
    }

    fn submit(&self, bytes: &[u8], width: usize, height: usize, stride: usize, time: f64) {
        let received = Instant::now();
        // One memcpy only; do not retain the native pixel buffer across callbacks.
        let frame = RawFrame {
            bytes: bytes.to_vec(),
            width,
            height,
            stride,
            time,
            received,
        };
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        if let Some(previous) = state.previous_time {
            let gap = (time - previous) * 1000.0;
            if gap > 150.0 {
                eprintln!("[perf] long native source_gap={gap:.0}ms copy={}ms raw_queued={} raw_dropped={}",
                    received.elapsed().as_millis(), state.frames.len(), state.dropped);
            }
        }
        state.previous_time = Some(time);
        if state.pending.is_none() && state.bytes + frame.bytes.len() <= Self::RAW_BUDGET {
            state.bytes += frame.bytes.len();
            state.frames.push_back(frame);
        } else if state.pending.replace(frame).is_some() {
            state.dropped += 1;
            self.output.state.lock().unwrap().dropped += 1;
        }
        self.ready.notify_one();
    }

    fn take(&self) -> Option<RawFrame> {
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(frame) = state.frames.pop_front() {
                state.bytes -= frame.bytes.len();
                if let Some(pending) = state.pending.take() {
                    state.bytes += pending.bytes.len();
                    state.frames.push_back(pending);
                }
                return Some(frame);
            }
            if let Some(frame) = state.pending.take() {
                return Some(frame);
            }
            if state.closed {
                return None;
            }
            state = self.ready.wait(state).unwrap();
        }
    }

    fn run(&self) {
        while let Some(frame) = self.take() {
            let started = Instant::now();
            self.output.receive_bgra_at(
                &frame.bytes,
                frame.width,
                frame.height,
                frame.stride,
                frame.time,
                frame.received,
            );
            if started.elapsed().as_millis() > 75 {
                let state = self.state.lock().unwrap();
                eprintln!(
                    "[perf] long ingress process={}ms age={}ms raw_queued={} raw_dropped={}",
                    started.elapsed().as_millis(),
                    frame.received.elapsed().as_millis(),
                    state.frames.len(),
                    state.dropped
                );
            }
        }
    }

    fn finish(&self, discard: bool) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        if discard {
            state.frames.clear();
            state.pending = None;
            state.bytes = 0;
        }
        self.ready.notify_all();
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
        let ingress = &*(context as *const Arc<FrameIngress>);
        let queue = &ingress.output;
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
        ingress.submit(
            std::slice::from_raw_parts(pixels, length),
            width,
            height,
            stride,
            time,
        );
    }

    pub(crate) struct MacCaptureStream {
        handle: *mut c_void,
        context: Box<Arc<FrameIngress>>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    // Native start/stop may run on workers. The output callback accesses only
    // the synchronized queue; native stop drains callbacks before freeing it.
    unsafe impl Send for MacCaptureStream {}

    impl MacCaptureStream {
        pub fn start(display: u32, rect: Rect, width: u32, height: u32) -> Result<Self, String> {
            let mut context = Box::new(Arc::new(FrameIngress::new(Arc::new(FrameQueue::new(
                width, height,
            )))));
            let worker_context = context.clone();
            let worker = std::thread::Builder::new()
                .name("long-frame-compress".into())
                .spawn(move || worker_context.run())
                .map_err(|e| e.to_string())?;
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
                    (&mut *context as *mut Arc<FrameIngress>).cast(),
                    error.as_mut_ptr(),
                    error.len(),
                )
            };
            if handle.is_null() {
                context.finish(true);
                let _ = worker.join();
                return Err(unsafe { CStr::from_ptr(error.as_ptr()) }
                    .to_string_lossy()
                    .into_owned());
            }
            eprintln!(
                "[snappin] mac long stream started: {}x{} 30fps async-ingress raw_budget=128MiB lossless queue={} frames (+ latest)",
                width, height, context.output.capacity
            );
            Ok(Self {
                handle,
                context,
                worker: Some(worker),
            })
        }

        pub fn next(&self) -> Result<Option<(StreamFrame, usize, u64)>, String> {
            self.context.output.pop()
        }

        pub fn stop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    snappin_long_stream_stop(self.handle);
                }
                self.handle = std::ptr::null_mut();
                self.context.finish(false);
                if let Some(worker) = self.worker.take() {
                    let _ = worker.join();
                }
                eprintln!("[snappin] mac long stream stopped");
            }
        }
    }

    impl Drop for MacCaptureStream {
        fn drop(&mut self) {
            self.context.finish(true);
            self.stop();
            self.context.output.close();
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
    fn ingress_copies_native_memory_and_drains_before_finishing() {
        let output = Arc::new(FrameQueue::new(2, 2));
        let ingress = Arc::new(FrameIngress::new(output.clone()));
        let mut bytes = [0u8; 16];
        // The native callback completes without running the compression worker.
        // SCK can reuse/mutate its buffer as soon as submit returns.
        for value in 1..=30u8 {
            bytes.fill(value);
            ingress.submit(&bytes, 2, 2, 8, value as f64 / 30.0);
        }
        bytes.fill(255);
        assert_eq!(ingress.state.lock().unwrap().frames.len(), 30);
        assert_eq!(output.state.lock().unwrap().sequence, 0);
        ingress.finish(false);
        let worker_ingress = ingress.clone();
        std::thread::spawn(move || worker_ingress.run())
            .join()
            .unwrap();
        for value in 1..=30u8 {
            let (frame, _, dropped) = output.pop().unwrap().unwrap();
            assert_eq!(frame.image.data[0], value);
            assert_eq!(frame.source_time, value as f64 / 30.0);
            assert_eq!(dropped, 0);
        }
        assert!(output.pop().unwrap().is_none());
    }

    #[test]
    fn cancelling_ingress_discards_raw_frames_and_late_callbacks() {
        let output = Arc::new(FrameQueue::new(2, 2));
        let ingress = FrameIngress::new(output.clone());
        ingress.submit(&[1; 16], 2, 2, 8, 1.0);
        ingress.finish(true);
        ingress.submit(&[2; 16], 2, 2, 8, 2.0);
        ingress.run();
        assert!(output.pop().unwrap().is_none());
        assert_eq!(ingress.state.lock().unwrap().bytes, 0);
    }

    #[test]
    fn ingress_retina_burst_keeps_overlap_through_a_paused_compressor() {
        use snappin_core_algo::stitch::{StitchOptions, StitchStepKind, Stitcher};
        let (width, frame_h, page_h) = (3344usize, 1676usize, 3000usize);
        let mut page = vec![0u8; width * page_h * 4];
        for y in 0..page_h {
            for x in 0..width {
                let n =
                    (y as u32).wrapping_mul(2654435761) ^ (x as u32 / 16).wrapping_mul(2246822519);
                let p = (y * width + x) * 4;
                page[p..p + 4].copy_from_slice(&[
                    (n >> 8) as u8,
                    (n >> 16) as u8,
                    (n >> 24) as u8,
                    255,
                ]);
            }
        }
        let first = RgbaBuffer::new(
            width as u32,
            frame_h as u32,
            page[..width * frame_h * 4].to_vec(),
        )
        .unwrap();
        let output = Arc::new(FrameQueue::new(width as u32, frame_h as u32));
        let ingress = Arc::new(FrameIngress::new(output.clone()));
        // Six raw Retina frames arrive while compression is completely paused.
        // The callback owns only a copy; later mutation cannot corrupt the frame.
        for i in 0..6 {
            let y = i * 200;
            let mut native = page[y * width * 4..(y + frame_h) * width * 4].to_vec();
            for pixel in native.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            ingress.submit(&native, width, frame_h, width * 4, i as f64 / 30.0);
            native.fill(0);
        }
        {
            let state = ingress.state.lock().unwrap();
            assert_eq!(state.dropped, 0);
            assert!(state.bytes <= FrameIngress::RAW_BUDGET);
        }
        assert_eq!(output.state.lock().unwrap().sequence, 0);
        ingress.finish(false);
        let worker_context = ingress.clone();
        std::thread::spawn(move || worker_context.run())
            .join()
            .unwrap();
        let mut st = Stitcher::new(&first, StitchOptions::for_frame(frame_h as u32)).unwrap();
        let mut count = 0;
        while let Some((frame, _, dropped)) = output.pop().unwrap() {
            assert_eq!(dropped, 0);
            assert_ne!(st.push(&frame.image).unwrap().kind, StitchStepKind::NoMatch);
            count += 1;
        }
        assert_eq!(count, 6);
        assert_eq!(st.height(), (1000 + frame_h) as u32);
        assert_eq!(st.canvas().data, page[..(1000 + frame_h) * width * 4]);
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
        use snappin_core_algo::stitch::{StitchOptions, StitchStepKind, Stitcher};
        let (width, height, frame_h) = (3344u32, 6500u32, 1676u32);
        let mut data = vec![0; width as usize * height as usize * 4];
        for y in 0..height {
            for x in 0..width {
                let n = y.wrapping_mul(2654435761) ^ (x / 16).wrapping_mul(2246822519);
                let p = ((y * width + x) * 4) as usize;
                data[p..p + 4].copy_from_slice(&[
                    (n >> 8) as u8,
                    (n >> 16) as u8,
                    (n >> 24) as u8,
                    255,
                ]);
            }
        }
        let crop = |y: u32| {
            RgbaBuffer::new(
                width,
                frame_h,
                data[(y * width * 4) as usize..((y + frame_h) * width * 4) as usize].to_vec(),
            )
            .unwrap()
        };
        let queue = FrameQueue::new(width, frame_h);
        // Model a consumer pause: all frames arrive before any matching runs.
        // The old three-frame queue dropped almost this entire burst.
        for i in 0..=30 {
            queue.push(crop(i * 150), i as f64 / 30.0);
        }
        {
            let state = queue.state.lock().unwrap();
            assert_eq!(state.dropped, 0);
            assert!(state.bytes <= QUEUE_BYTES);
            eprintln!(
                "31 Retina frames retained losslessly in {} bytes",
                state.bytes
            );
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
        assert_eq!(
            st.canvas().data,
            data[..((4500 + frame_h) * width * 4) as usize]
        );
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
