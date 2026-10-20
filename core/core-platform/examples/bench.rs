//! 基准测试：拆解截图链路各环节耗时（开发期诊断工具）。
//! 运行：cargo run -p snappin-core-platform --example bench

use std::time::Instant;

fn main() {
    // 预热（首次 DXGI 初始化可能偏慢）
    let _ = snappin_core_platform::capture_all_monitors();

    for round in 1..=3 {
        let t0 = Instant::now();
        let shots = snappin_core_platform::capture_all_monitors().expect("capture all");
        let total = t0.elapsed();
        let info: Vec<String> = shots
            .iter()
            .map(|s| format!("{} {}x{} png={}B", s.id, s.width, s.height, s.path.metadata().map(|m| m.len()).unwrap_or(0)))
            .collect();
        println!("[bench] round {}: total={:?} | {}", round, total, info.join(" | "));
    }
}
