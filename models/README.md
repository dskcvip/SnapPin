# OCR 模型托管说明

SnapPin 的 OCR 功能（截图取字）基于 **PaddleOCR PP-OCRv4 mobile** 模型，
经 [rten](https://github.com/potassium-oxide/rten)（纯 Rust ONNX 运行时）本地推理。
模型文件共约 **15.6 MB**，本目录即为 git 托管的权威副本——clone 仓库后即可直接使用，
**无需任何下载步骤**。

## 文件清单与校验值

| 文件 | 大小 | SHA256 | 用途 |
|---|---|---|---|
| `det.onnx` | 4,745,517 B (4.7 MB) | `d2a7720d45a54257208b1e13e36a8479894cb74155a5efe29462512d42f49da9` | DBNet 文本检测，输入 `[1,3,H,W]`，输出 `[1,1,H,W]` 概率图 |
| `rec.onnx` | 10,857,958 B (10.4 MB) | `48fc40f24f6d2a207a2b1091d3437eb3cc3eb6b676dc3ef9c37384005483683b` | CRNN 文本识别，输入 `[1,3,48,320]`，输出 `[1,40,6625]` CTC 概率 |
| `ppocr_keys_v1.txt` | 26,249 B (26 KB) | `28b2362ad4ab2dc38769aa72feb535e3a9ddb3fd2a7585a05920e6393b1dc7f7` | 6623 个字典字符（UTF-8，一行一个），配合 CTC 解码 |

> 校验命令：`sha256sum models/ocr/*`

## 来源与许可

| 上游 | 地址 | 许可 |
|---|---|---|
| 模型权重 | ModelScope [`RapidAI/RapidOCR`](https://www.modelscope.cn/models/RapidAI/RapidOCR) | Apache-2.0 |
| 原始项目 | [PaddleOCR](https://github.com/PaddlePaddle/PaddleOCR) PP-OCRv4 mobile | Apache-2.0 |

模型按 Apache-2.0 许可分发，可自由商用；上游未对导出权重附加额外限制。
SnapPin 仅做格式转换（Paddle Inference → ONNX，由 RapidOCR 仓库提供），未改动权重。

### 直链（已验证，2026-10-21 下载比对 SHA256 一致）

```
det.onnx:
https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.5.0/onnx/PP-OCRv4/det/ch_PP-OCRv4_det_infer.onnx

rec.onnx:
https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v3.5.0/onnx/PP-OCRv4/rec/ch_PP-OCRv4_rec_infer.onnx

ppocr_keys_v1.txt:
https://www.modelscope.cn/models/RapidAI/RapidOCR/resolve/v2.0.7/paddle/PP-OCRv4/rec/ch_PP-OCRv4_rec_infer/ppocr_keys_v1.txt
```

> 注意：det/rec 挂在 v3.5.0 tag 下，字典文件在 v2.0.7 tag 下（RapidOCR 仓库的历史结构），
> 三者版本组合经 SHA256 验证与本目录一致。若上游调整 tag 导致直链失效，
> 以本目录的 SHA256 为准重新核对，或到 RapidOCR 仓库按路径查找。

## 目录约定（运行期）

运行时模型目录解析优先级（见 `src-tauri/src/commands.rs::resolve_models_dir`）：

1. **`SNAPPIN_MODELS_DIR` 环境变量** —— 手动覆盖，指向包含三件套的目录；
2. **debug 构建** —— 自动使用仓库内的 `models/ocr/`（即本目录，开发零配置）；
3. **release 构建** —— `%APPDATA%/com.snappin.dev/models/ocr/`（Windows）/
   `~/Library/Application Support/com.snappin.dev/models/ocr/`（macOS）。

也就是说：**开发者 clone 后 debug 直接跑**；普通用户安装 release 包后，
首次使用 OCR 时若检测到模型缺失，工具条 OCR 按钮会带红点提示，
按 `scripts/fetch_models.py` 下载即可落到固定目录。

## 重新获取 / 修复

```bash
# 下载到默认目录（%APPDATA%/com.snappin.dev/models/ocr/）并校验 SHA256
python scripts/fetch_models.py

# 或指定目录
python scripts/fetch_models.py --out /path/to/models/ocr
```

脚本只做三件事：流式下载 → 校验 SHA256 → 失败时给出明确错误。
**校验不过不会留下半成品**（先写 `.part` 临时文件，通过后才原子改名）。

## 为什么进 git 而不是按需下载

用户（项目所有者）明确要求"模型按需下载到一个固定的目录，后边上传到 git，
方便用户下载"。15.6 MB 在 GitHub 单文件 100 MB 限制内，直接入库：
clone 即用、版本与代码绑定、CI/新机器零额外步骤。
若未来模型升级导致体积膨胀，再考虑 Git LFS 或 GitHub Releases 分发。
