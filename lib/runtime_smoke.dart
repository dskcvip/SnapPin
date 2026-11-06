import 'dart:async';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import 'app_log.dart';
import 'ocr_bridge.dart';
import 'long_stitch.dart';
import 'long_stitch_worker.dart';
import 'owned_images.dart';

import 'package:image/image.dart' as img;

/// Explicit release verification mode, independent of user configuration,
/// hotkeys, tray, screen permissions and the normal single-instance mutex.
Future<void> runRuntimeSmoke({required bool shutdownDuringOcr}) async {
  if (Platform.isWindows && !shutdownDuringOcr) {
    final frames = await const MethodChannel('snappin/capture')
        .invokeListMethod<Uint8List>('longCaptureSmoke');
    if (frames == null || frames.length != 2) {
      throw StateError('Missing consecutive capture frames');
    }
    for (final frame in frames) {
      final image = img.decodePng(frame)!;
      if (image.width != 260 || image.height != 320) {
        throw StateError('Consecutive capture dimensions changed');
      }
      for (final y in [5, 160, 315]) {
        for (final x in [5, 130, 255]) {
          final pixel = image.getPixel(x, y);
          if ((pixel.r - 37).abs() > 1 ||
              (pixel.g - 157).abs() > 1 ||
              (pixel.b - 211).abs() > 1) {
            throw StateError(
              'Hidden/reused capture surface obscured source pixels',
            );
          }
        }
      }
    }
    AppLog.write(
      'runtime-smoke-long-repeat-ready',
      'cycles=2 originalPixels=true',
    );
  }
  if (Platform.isMacOS) {
    final status = await const MethodChannel('snappin/capture')
        .invokeMapMethod<String, dynamic>('capturePermissionStatus');
    if (status?['bundleId'] != 'com.snappin.dev' ||
        status?['preflightGranted'] is! bool ||
        status?['accessibilityGranted'] is! bool ||
        status?['signatureStatus'] != 0 ||
        status?['designatedRequirement'] is! String ||
        !(status!['bundlePath'] as String).endsWith('/SnapPin.app')) {
      throw StateError('Native permission diagnostic failed: $status');
    }
    AppLog.write('runtime-smoke-permission-status-ready', status);
    if (!shutdownDuringOcr) {
      final capture = await const MethodChannel('snappin/capture')
          .invokeMapMethod<String, dynamic>('longCaptureSmoke');
      final frames = (capture!['frames'] as List).cast<Uint8List>();
      final positions = (capture['positions'] as List).cast<int>();
      final rawFrames = (capture['rawFrames'] as List).cast<Map>();
      final rawFirst = rawFrames.first;
      final rawWorker = await LongStitchWorker.start(
        rawFirst['rgba'] as Uint8List,
        rawWidth: rawFirst['width'] as int,
        rawHeight: rawFirst['height'] as int,
      );
      final session = LongStitchSession.start(frames.first);
      try {
        final first = img.decodePng(frames.first)!;
        // Same CGImage through PNG and RGBA: catches inverted rows, swapped
        // channels, unexpected stride or accidental resolution changes.
        for (var index = 0; index < frames.length; index++) {
          final decoded = img.decodePng(frames[index])!;
          final raw = rawFrames[index];
          final pixels = raw['rgba'] as Uint8List;
          if (raw['width'] != decoded.width ||
              raw['height'] != decoded.height ||
              pixels.length != decoded.width * decoded.height * 4) {
            throw StateError('Native raw frame dimensions changed');
          }
          for (var y = 0; y < decoded.height; y += 7) {
            for (var x = 0; x < decoded.width; x += 11) {
              final pixel = decoded.getPixel(x, y);
              final offset = (y * decoded.width + x) * 4;
              if ((pixel.r - pixels[offset]).abs() > 16 ||
                  (pixel.g - pixels[offset + 1]).abs() > 16 ||
                  (pixel.b - pixels[offset + 2]).abs() > 16 ||
                  pixels[offset + 3] != 255) {
                throw StateError(
                  'Native raw/PNG pixel mismatch frame=$index x=$x y=$y',
                );
              }
            }
          }
        }
        final scale = first.height / (capture['logicalHeight'] as int);
        for (var index = 1; index < frames.length; index++) {
          final step = session.push(frames[index]);
          final raw = rawFrames[index];
          final rawStep = await rawWorker.pushRgba(
            raw['rgba'] as Uint8List,
            raw['width'] as int,
            raw['height'] as int,
          );
          final expected =
              ((positions.take(index + 1).reduce((a, b) => a > b ? a : b) +
                          (capture['logicalHeight'] as int)) *
                      scale)
                  .round();
          if (rawStep.kind != step.kind ||
              rawWorker.height != session.height ||
              rawWorker.width != session.width ||
              step.kind == LongStepKind.noMatch ||
              (session.height - expected).abs() > 8) {
            throw StateError(
              'Native scrolling did not stitch: index=$index '
              'kind=${step.kind} height=${session.height} expected=$expected',
            );
          }
        }
        if ((capture['wheelEvents'] as int) < frames.length - 1) {
          throw StateError('Native wheel events were not delivered');
        }
        AppLog.write(
          'runtime-smoke-long-raw-ready',
          'frames=${rawFrames.length} output=${rawWorker.width}x${rawWorker.height} pixelsAndDirectionVerified=true',
        );
        AppLog.write(
          'runtime-smoke-long-capture-ready',
          'frames=${frames.length} output=${session.width}x${session.height} scale=$scale',
        );
      } finally {
        rawWorker.dispose();
        session.dispose();
      }
    }
  }
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
  if (Platform.isMacOS && !shutdownDuringOcr) {
    final directory = await OwnedImages.create();
    final file = File('${directory.path}/print.png');
    try {
      await file.writeAsBytes(bytes);
      final prepared = await const MethodChannel('snappin/capture')
          .invokeMethod<bool>('printImage', file.path);
      if (prepared != true) throw StateError('Native print job setup failed');
      AppLog.write('runtime-smoke-print-prepared');
    } finally {
      await OwnedImages.delete(directory.path, [file.path]);
    }
  }
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
    AppLog.flush();
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
  AppLog.flush();
  await windowManager.close();
  if (Platform.isMacOS) exit(0);
}
