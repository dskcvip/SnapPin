import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import 'app_log.dart';
import 'ocr_bridge.dart';

/// Explicit release verification mode, independent of user configuration,
/// hotkeys, tray, screen permissions and the normal single-instance mutex.
Future<void> runRuntimeSmoke({required bool shutdownDuringOcr}) async {
  await windowManager.setAsFrameless();
  for (final value in [true, false, true]) {
    await windowManager.setHasShadow(value);
    if (await windowManager.hasShadow() != value) {
      throw StateError('Window shadow did not change to $value');
    }
  }
  AppLog.write('runtime-smoke-shadow-ready');
  final recorder = ui.PictureRecorder();
  final canvas = ui.Canvas(recorder);
  canvas.drawColor(const ui.Color(0xffffffff), ui.BlendMode.src);
  final paragraph =
      (ui.ParagraphBuilder(ui.ParagraphStyle(fontSize: 32))
            ..pushStyle(ui.TextStyle(color: const ui.Color(0xff000000)))
            ..addText('SnapPin 12345 local OCR'))
          .build()
        ..layout(const ui.ParagraphConstraints(width: 1000));
  for (var y = 10.0; y < 3950; y += 50) {
    canvas.drawParagraph(paragraph, ui.Offset(20, y));
  }
  final picture = recorder.endRecording();
  final image = await picture.toImage(1000, 4000);
  final bytes = (await image.toByteData(format: ui.ImageByteFormat.png))!.buffer
      .asUint8List();
  image.dispose();
  picture.dispose();
  paragraph.dispose();
  var completed = false;
  final recognition = OcrBridge.recognizeLayout(bytes)
      .timeout(const Duration(seconds: 60));
  recognition.then<void>(
    (_) {
      completed = true;
    },
    onError: (Object error) {
      completed = true;
    },
  );
  await Future<void>.delayed(const Duration(milliseconds: 30));
  final watch = Stopwatch()..start();
  await windowManager.getTitle();
  if (completed || watch.elapsedMilliseconds >= 1000) {
    throw StateError(
      'Platform thread blocked during OCR (${watch.elapsedMilliseconds}ms, completed=$completed)',
    );
  }
  AppLog.write(
    'runtime-smoke-ocr-responsive',
    '${watch.elapsedMilliseconds}ms',
  );
  if (shutdownDuringOcr) {
    recognition.ignore();
    AppLog.write('runtime-smoke-shutdown-start');
    await windowManager.close();
    if (Platform.isMacOS) exit(0);
    return;
  }
  if (Platform.isWindows || Platform.isMacOS) {
    try {
      await OcrBridge.recognize(bytes);
      throw StateError('Concurrent OCR was not rejected');
    } on PlatformException catch (error) {
      if (error.code != 'ocr_busy') rethrow;
    }
    AppLog.write('runtime-smoke-ocr-busy-ready');
  }
  final document = await recognition;
  if (!document.hasLayout ||
      !document.text.contains('12345') ||
      document.lines.any(
        (line) =>
            line.bounds.left < 0 ||
            line.bounds.top < 0 ||
            line.bounds.right > document.width + 1 ||
            line.bounds.bottom > document.height + 1,
      )) {
    throw StateError('OCR layout or fixture text is invalid');
  }
  AppLog.write(
    'runtime-smoke-ocr-layout-ready',
    'lines=${document.lines.length}',
  );
  try {
    await OcrBridge.recognize(Uint8List.fromList([1, 2, 3]));
    throw StateError('Invalid OCR input was not rejected');
  } on PlatformException {
    AppLog.write('runtime-smoke-ocr-error-ready');
  }
  await windowManager.getTitle();
  AppLog.write('runtime-smoke-ready');
  await windowManager.close();
  if (Platform.isMacOS) exit(0);
}
