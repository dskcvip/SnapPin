//! 运行期日志落盘。
//!
//! ## 为什么需要
//!
//! release 构建带 `windows_subsystem = "windows"`（见 main.rs），进程没有控制台，
//! 所有 `eprintln!` 都被写进一个无效句柄——**静默丢弃**。于是用户反馈
//! "按 Ctrl+1 没反应"时，我们手上没有任何证据区分：
//!
//! 1. 热键根本没注册上（被别的程序占用 / RegisterHotKey 失败）；
//! 2. 热键注册上了但插件回调没触发；
//! 3. 回调触发了、`do_capture` 中途失败（采集/建窗/信号量）。
//!
//! 这里在启动最早期把 **stderr 整个重定向**到
//! `%APPDATA%\com.snappin.dev\snappin.log`（Windows；macOS 为
//! `~/Library/Application Support/com.snappin.dev/snappin.log`），
//! 这样既有的、遍布 `commands.rs` 的 `eprintln!` 不用改一行就全部落盘。
//! 另有 `logln!` 给"没有 eprintln 但必须留痕"的事件（热键回调、托盘点击）埋点。
//!
//! ## 实现要点
//!
//! - **句柄必须比进程活得久**：文件用 `Box::leak` 泄漏成 `'static`，再交给
//!   `SetStdHandle`；否则 drop 后 stderr 指向野句柄。
//! - std 的 `Stderr` 每次 write 都 `GetStdHandle(STD_ERROR_HANDLE)`，
//!   所以早期 `SetStdHandle` 对随后所有 `eprintln!` 都生效。
//! - 两个句柄（重定向用的、`logln!` 用的）都按 append 打开，Windows 上
//!   追加写由 `FILE_APPEND_DATA` 保证原子，不会互相覆盖。
//! - 单文件超过上限就**截断重开**（不滚动备份：目标读者是排障，只看最近一次）。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// 日志文件上限。单次运行正常在几十 KB 量级，2 MB 足够容纳异常刷屏。
const MAX_BYTES: u64 = 2 * 1024 * 1024;

static LOG_FILE: OnceLock<Mutex<Option<File>>> = OnceLock::new();
static SINCE_START: OnceLock<Instant> = OnceLock::new();

/// 日志所在目录：与 Tauri 的 `app_data_dir()` 保持一致
/// （Windows `%APPDATA%\{identifier}`）。
fn log_dir() -> PathBuf {
    if let Some(base) = std::env::var_os("APPDATA") {
        return PathBuf::from(base).join("com.snappin.dev");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join("Library/Application Support/com.snappin.dev");
    }
    std::env::temp_dir().join("com.snappin.dev")
}

/// 日志文件的绝对路径（供 `app_info` / 前端"打开日志"用）。
pub fn log_path() -> PathBuf {
    log_dir().join("snappin.log")
}

/// 启动最早调用一次：开文件、写启动标记、把 stderr 指过去。
///
/// 失败时**静默降级**——日志不该成为启动失败的理由。
pub fn init() {
    let _ = SINCE_START.set(Instant::now());
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = log_path();

    let too_big = std::fs::metadata(&path)
        .map(|m| m.len() > MAX_BYTES)
        .unwrap_or(false);

    let mut opts = OpenOptions::new();
    opts.create(true).write(true);
    if too_big {
        opts.truncate(true);
    } else {
        opts.append(true);
    }
    let file = match opts.open(&path) {
        Ok(f) => f,
        Err(_) => return,
    };

    // stderr 用一份独立句柄，并泄漏成 'static（见模块文档）
    if let Ok(clone) = file.try_clone() {
        redirect_stderr(clone);
    }
    let _ = LOG_FILE.set(Mutex::new(Some(file)));

    crate::logln!("===== SnapPin {} start =====", env!("CARGO_PKG_VERSION"));
    crate::logln!("log file: {}", path.display());
}

/// 往日志追加一行（带自启动以来的秒数，便于对齐时序）。
pub fn write_line(msg: &str) {
    let secs = SINCE_START.get().map(|t| t.elapsed().as_secs_f64()).unwrap_or(0.0);
    let cell = LOG_FILE.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        if let Some(f) = guard.as_mut() {
            // 忽略写失败：日志不影响功能
            let _ = writeln!(f, "[{secs:8.3}s] {msg}");
            let _ = f.flush();
        }
    }
}

/// 追加一行日志。用法同 `println!`。
///
/// 在子模块里用 `crate::logln!(...)`。
#[macro_export]
macro_rules! logln {
    ($($arg:tt)*) => {
        $crate::logfile::write_line(&format!($($arg)*))
    };
}

#[cfg(windows)]
fn redirect_stderr(file: File) {
    use std::os::windows::io::AsRawHandle;

    // (DWORD)-12：Windows SDK 的 STD_ERROR_HANDLE
    const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;

    unsafe extern "system" {
        fn SetStdHandle(n_std_handle: u32, h_handle: *mut core::ffi::c_void) -> i32;
    }

    let leaked: &'static File = Box::leak(Box::new(file));
    unsafe {
        SetStdHandle(
            STD_ERROR_HANDLE,
            leaked.as_raw_handle() as *mut core::ffi::c_void,
        );
    }
}

#[cfg(not(windows))]
fn redirect_stderr(file: File) {
    use std::os::fd::AsRawFd;

    unsafe extern "C" {
        fn dup2(oldfd: i32, newfd: i32) -> i32;
    }

    let leaked: &'static File = Box::leak(Box::new(file));
    unsafe {
        // 2 == STDERR_FILENO
        dup2(leaked.as_raw_fd(), 2);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_path_is_absolute_and_named() {
        let p = log_path();
        assert!(p.is_absolute(), "log path should be absolute: {p:?}");
        assert_eq!(p.file_name().unwrap(), "snappin.log");
    }

    #[test]
    fn write_line_without_init_does_not_panic() {
        // init() 未调用时 LOG_FILE 是 None：必须静默丢弃，不能 panic/阻塞
        write_line("test line before init");
    }
}
