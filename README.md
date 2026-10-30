# SnapPin Flutter

This is the standalone Flutter rewrite of SnapPin. The existing Tauri app in
`E:\PixPin` is the behavior and visual reference until parity is verified.
The Flutter application is under construction and is not a replacement build yet.

## Parity Gate

Each row needs functional, visual, keyboard, multi-monitor, and platform checks
before the Flutter build replaces the current application. The source of truth is
the current app, especially `docs/FEATURES.md`, `docs/CONTRACTS.md`, and the
corresponding React window and CSS files.

| Window / workflow | Reference | Status |
| --- | --- | --- |
| Floating ball and menu | `BallWindow`, `MenuWindow` | Implemented: floating ball, tray and shared menu with shortcut conflict warnings; native visual QA remains |
| Frozen screenshot and selection | `OverlayWindow` | Partial: multi-monitor capture, crop, window hover and click-to-select, eight resize handles, magnifier, color copy; visual and interaction QA remain |
| Annotation tools and properties | `AnnotationLayer`, `ToolProps` | Partial: all toolbar entries, local sticker assets and Dart effects are wired; interaction and visual QA remain |
| Image pin, context menu, groups | `PinWindow`, `PinManagerWindow` | Partial: pin/copy/save/scale/opacity/lock, context menu, image processing and groups are wired; visual and interaction QA remain |
| OCR and QR results | `OcrWindow` | Partial: local Windows OCR / macOS Vision, OCR image-and-text window, Dart multi-code decoding; visual QA remains |
| Translation | `TranslationWindow` | Partial: local OCR before text-only model translation; visual QA remains |
| Long screenshot and preview | `LongResultWindow`, `LongPreviewWindow` | Implemented: manual scroll, background Dart stitching, full resolution output, preview and shared pin viewer; real browser regression remains |
| Settings, shortcuts, models, data | `MainWindow` | Implemented: Tauri layout and model workflows, suspend/resume during shortcut recording; native visual and workflow QA remain |
| Tray, single instance, startup | `src-tauri/src/lib.rs` | Partial: tray and shortcuts, Windows single-instance guard; macOS lifecycle and startup QA remain |

System APIs needed for screen capture, window control, shortcuts, and the tray
are accessed through desktop plugins or small platform channels. UI and app
behavior, long-image stitching, and annotation effects belong in Flutter/Dart;
the Tauri and Rust code is not linked into this project.

OCR runs on the user's computer through Windows.Media.Ocr or macOS Vision.
Screenshots are never sent to an OCR service. Windows recognition requires the
appropriate OCR language to be installed in the operating system. Translation
uses the model endpoint configured by the user and sends only recognized text.

## Development

Use the Flutter SDK at `E:\huanjing\devtools\flutter`:

```powershell
& 'E:\huanjing\devtools\flutter\bin\flutter.bat' pub get
& 'E:\huanjing\devtools\flutter\bin\flutter.bat' analyze
& 'E:\huanjing\devtools\flutter\bin\flutter.bat' build windows
```

macOS builds require a macOS host or a GitHub Actions macOS runner. UI smoothness,
screen recording permission, and multi-monitor behavior still require testing on
a real Mac.

The Flutter source has its own `.github/workflows/build-macos-dmg.yml`. It can
produce an ad-hoc-signed DMG after this standalone project is committed and
pushed to a GitHub repository. The existing `E:\PixPin` workflow still builds
the Tauri application; it does not include the Flutter source on drive F.
