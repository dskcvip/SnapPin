fn main() {
    // 图标变更必须触发本脚本重跑 —— tauri-build 不会自动声明这个依赖。
    //
    // 症状（2026-10-25 用户反馈）：重新生成 `icons/` 之后窗口/托盘图标都变了，
    // 但 exe 文件上的图标还是旧的 —— 因为 Windows 图标资源是在**链接阶段**
    // 由构建脚本嵌入的，图标变了而脚本没重跑，资源就不更新。
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    tauri_build::build()
}
