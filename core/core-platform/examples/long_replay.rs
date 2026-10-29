//! Replay browser-exported JPEG viewport frames plus positions.json.
//! Run with: cargo run -p snappin-core-platform --release --example long_replay -- <fixture-dir>
//! This checks the shared stitcher, not OS wheel input or native capture.
use snappin_core_algo::{
    stitch::{StitchOptions, StitchStepKind, Stitcher},
    Rect,
};
use snappin_core_platform::{load_rgba, save_png_fast};
use std::path::PathBuf;
fn main() {
    let dir = PathBuf::from(std::env::args().nth(1).expect("fixture directory"));
    let positions: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("positions.json")).unwrap()).unwrap();
    let mut st: Option<Stitcher> = None;
    let mut max_y = 0.0f64;
    let mut min_y = f64::MAX;
    let mut fh = 0;
    for (index, row) in positions.as_array().unwrap().iter().enumerate() {
        let img = load_rgba(&dir.join(row["file"].as_str().unwrap())).unwrap();
        let frame = img
            .crop(&Rect {
                x: 202,
                y: 54,
                w: 840,
                h: 650,
            })
            .unwrap();
        let y = row["y"].as_f64().unwrap();
        let already_captured = y >= min_y && y <= max_y;
        max_y = max_y.max(y);
        min_y = min_y.min(y);
        fh = frame.height;
        if let Some(st) = st.as_mut() {
            let step = st.push(&frame).unwrap();
            println!(
                "{} y={y} {:?} added={} h={} shift={} mean={}",
                row["file"], step.kind, step.appended, step.height, step.best_shift, step.best_mean
            );
            assert_ne!(step.kind, StitchStepKind::NoMatch);
            if already_captured {
                assert_eq!(step.appended, 0, "returning to captured content must not duplicate rows");
            }
            let expected = fh + ((max_y - min_y) * 712.0 / 720.0).round() as u32;
            assert!(step.height.abs_diff(expected) <= index as u32 + 1,
                "frame {index}: height {}, expected {expected}", step.height);
        } else {
            st = Some(Stitcher::new(&frame, StitchOptions::for_frame(frame.height)).unwrap());
        }
    }
    let st = st.unwrap();
    let expected = fh + ((max_y - min_y) * 712.0 / 720.0).round() as u32;
    assert!(
        st.height().abs_diff(expected) <= positions.as_array().unwrap().len() as u32,
        "height {}, expected {expected}",
        st.height()
    );
    save_png_fast(st.canvas(), &dir.join("stitched-browser.png")).unwrap();
    println!(
        "Browser replay passed: {}x{}, expected height {expected}",
        st.canvas().width,
        st.height()
    );
}
