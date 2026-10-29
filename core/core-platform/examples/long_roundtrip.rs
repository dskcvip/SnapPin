//! Replay a rendered long page through the production stitcher. This isolates
//! alignment from OS input/capture and compares every output pixel with source.
//! cargo run -p snappin-core-platform --release --example long_roundtrip -- <page-image> <output-dir>
use snappin_core_algo::{stitch::{StitchOptions, StitchStepKind, Stitcher}, Rect};
use snappin_core_platform::{load_rgba, save_png_fast};
use std::path::PathBuf;

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let input = load_rgba(&PathBuf::from(&args[1])).unwrap();
    let dir = PathBuf::from(&args[2]);
    std::fs::create_dir_all(&dir).unwrap();
    // long-scroll.html: 850px main column, 52px sticky header.
    let page = input.crop(&Rect { x: (input.width as i32 - 850) / 2,
        y: 52, w: 850, h: input.height - 52 }).unwrap();
    for (name, height, strides) in [
        ("normal", 700, vec![120, 240, 80, 360]),
        ("fast", 700, vec![540, 620, 411, 651]),
        // Retain enough overlap to include actual page detail. A 30px strip
        // can be entirely blank on this page and cannot establish a position.
        ("short-selection", 500, vec![430, 311, 420, 200]),
    ] {
        let last = page.height - height;
        let initial = 30 * 160;
        let crop = |y: u32| page.crop(&Rect { x: 0, y: y as i32, w: page.width, h: height }).unwrap();
        let mut st = Stitcher::new(&crop(initial), StitchOptions::for_frame(height)).unwrap();
        let mut positions = Vec::new();
        let mut y = initial;
        let mut n = 0;
        while y < last { y = (y + strides[n % strides.len()]).min(last); positions.push(y); n += 1; }
        // Skip many already captured screens in one reverse movement.
        y = initial + 800;
        positions.push(y);
        while y > 0 { y = y.saturating_sub(strides[n % strides.len()]); positions.push(y); n += 1; }
        // Repeated direction changes must leave already captured pixels alone.
        positions.extend([height - 50, 0, 2 * height, height, 0]);
        let (mut top, mut bottom) = (initial, initial + height);
        let started = std::time::Instant::now();
        for (index, y) in positions.iter().copied().enumerate() {
            let step = st.push(&crop(y)).unwrap();
            assert_ne!(step.kind, StitchStepKind::NoMatch, "{name} frame {index} y={y} {step:?}");
            top = top.min(y); bottom = bottom.max(y + height);
            assert_eq!(st.height(), bottom - top, "{name} frame {index} y={y} {step:?}");
            let expected = page.crop(&Rect { x: 0, y: top as i32, w: page.width, h: bottom - top }).unwrap();
            assert!(st.canvas().data == expected.data, "{name} pixels frame {index} y={y}");
        }
        assert_eq!(st.canvas(), &page);
        save_png_fast(st.canvas(), &dir.join(format!("{name}.png"))).unwrap();
        println!("{name}: {} frames, {}x{}, every pixel correct, {:?}", positions.len(), page.width, page.height, started.elapsed());
    }
}
