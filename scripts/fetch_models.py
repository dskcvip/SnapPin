#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SnapPin OCR 模型下载器（PP-OCRv4 mobile → rten）。

从 ModelScope（RapidAI/RapidOCR）下载三件套到固定目录并校验 SHA256：
    det.onnx / rec.onnx / ppocr_keys_v1.txt

目标目录默认与 Tauri 的 app_data_dir() 一致（见 src-tauri/src/commands.rs）：
    Windows : %APPDATA%/com.snappin.dev/models/ocr
    macOS   : ~/Library/Application Support/com.snappin.dev/models/ocr
    Linux   : ~/.local/share/com.snappin.dev/models/ocr

用法：
    python scripts/fetch_models.py                  # 下载到默认目录
    python scripts/fetch_models.py --out DIR        # 指定目录
    python scripts/fetch_models.py --check          # 只校验不下载
    python scripts/fetch_models.py --force          # 已存在也重新下载

零第三方依赖（仅标准库）。校验不过不落盘：先写 .part，通过后才原子改名。
"""
import argparse
import hashlib
import os
import sys
import tempfile
import urllib.request

# ---------------------------------------------------------------- 常量

BASE = "https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve"

FILES = [
    # (文件名, URL, 字节数, SHA256)
    (
        "det.onnx",
        f"{BASE}/v3.5.0/onnx/PP-OCRv4/det/ch_PP-OCRv4_det_infer.onnx",
        4_745_517,
        "d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9",
    ),
    (
        "rec.onnx",
        f"{BASE}/v3.5.0/onnx/PP-OCRv4/rec/ch_PP-OCRv4_rec_infer.onnx",
        10_857_958,
        "48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b",
    ),
    (
        "ppocr_keys_v1.txt",
        f"{BASE}/v2.0.7/paddle/PP-OCRv4/rec/ch_PP-OCRv4_rec_infer/ppocr_keys_v1.txt",
        26_249,
        "28b2362ad4ab2dc38769aa72feb535e3a9ddb3fd2a7585a05920e6393b1dc7f7",
    ),
]

APP_ID = "com.snappin.dev"


def default_out_dir():
    """与 Tauri v2 app_data_dir() 对齐的平台默认目录。"""
    if sys.platform.startswith("win"):
        base = os.environ.get("APPDATA", os.path.expanduser("~/AppData/Roaming"))
        return os.path.join(base, APP_ID, "models", "ocr")
    if sys.platform == "darwin":
        return os.path.expanduser(f"~/Library/Application Support/{APP_ID}/models/ocr")
    base = os.environ.get("XDG_DATA_HOME", os.path.expanduser("~/.local/share"))
    return os.path.join(base, APP_ID, "models", "ocr")


# ---------------------------------------------------------------- 工具

def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(1 << 20)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def human(n):
    for unit in ("B", "KB", "MB", "GB"):
        if n < 1024 or unit == "GB":
            return f"{n:.1f} {unit}" if unit != "B" else f"{n} B"
        n /= 1024


def download(url, dest, expect_sha, expect_size):
    """流式下载到 dest.part，校验 SHA256/字节数，通过后原子改名。

    返回 None 表示成功；返回字符串表示失败原因。
    """
    tmp = dest + ".part"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "SnapPin-model-fetcher/1.0"})
        with urllib.request.urlopen(req, timeout=60) as resp:
            total = int(resp.headers.get("Content-Length") or 0)
            done = 0
            with open(tmp, "wb") as f:
                while True:
                    chunk = resp.read(1 << 20)
                    if not chunk:
                        break
                    f.write(chunk)
                    done += len(chunk)
                    if total:
                        pct = done * 100 // total
                        print(f"\r  {human(done)} / {human(total)} ({pct}%)", end="", flush=True)
            print()
    except Exception as e:  # 网络错误：不留残局
        try:
            os.remove(tmp)
        except OSError:
            pass
        return f"下载失败: {e}"

    if expect_size and os.path.getsize(tmp) != expect_size:
        try:
            os.remove(tmp)
        except OSError:
            pass
        return f"字节数不符: 期望 {expect_size}，实得 {os.path.getsize(tmp)}"

    got = sha256_of(tmp)
    if got != expect_sha:
        try:
            os.remove(tmp)
        except OSError:
            pass
        return f"SHA256 不符:\n    期望 {expect_sha}\n    实得 {got}"

    os.replace(tmp, dest)  # 同卷原子改名
    return None


# ---------------------------------------------------------------- 主流程

def main():
    ap = argparse.ArgumentParser(description="SnapPin OCR 模型下载器")
    ap.add_argument("--out", default=None, help="目标目录（默认平台 app_data_dir）")
    ap.add_argument("--force", action="store_true", help="已存在也重新下载")
    ap.add_argument("--check", action="store_true", help="只校验现有文件，不下载")
    args = ap.parse_args()

    out_dir = os.path.abspath(args.out) if args.out else default_out_dir()
    print(f"目标目录: {out_dir}")

    if not args.check:
        os.makedirs(out_dir, exist_ok=True)

    failures = []
    for name, url, size, sha in FILES:
        dest = os.path.join(out_dir, name)

        if os.path.isfile(dest) and not args.force and not args.check:
            if sha256_of(dest) == sha:
                print(f"[跳过] {name} 已存在且校验通过")
                continue
            print(f"[重下] {name} 已存在但校验不过")

        if args.check:
            if not os.path.isfile(dest):
                print(f"[缺失] {name}")
                failures.append(name)
            elif sha256_of(dest) != sha:
                print(f"[损坏] {name}（SHA256 不符）")
                failures.append(name)
            else:
                print(f"[ OK ] {name}")
            continue

        print(f"[下载] {name} ({human(size)})")
        err = download(url, dest, sha, size)
        if err:
            print(f"[失败] {name}: {err}")
            failures.append(name)
        else:
            print(f"[完成] {name} 校验通过")

    if failures:
        print(f"\n{len(failures)} 个文件未就绪: {', '.join(failures)}")
        print("请检查网络（ModelScope 访问）后重试；--check 可复验。")
        return 1
    print("\n全部就绪。启动 SnapPin 后工具条 OCR 按钮即可使用（release 包同样读此目录）。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
