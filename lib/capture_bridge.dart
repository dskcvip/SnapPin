import 'dart:io';

import 'package:flutter/services.dart';

class CapturedDisplay {
  const CapturedDisplay({
    required this.path,
    required this.x,
    required this.y,
    required this.width,
    required this.height,
    required this.pixelWidth,
    required this.pixelHeight,
    required this.scale,
    required this.pixelX,
    required this.pixelY,
  });

  final String path;
  final double x, y, width, height, scale;
  final int pixelWidth, pixelHeight;
  final int pixelX, pixelY;

  factory CapturedDisplay.fromMap(Map<String, dynamic> value) =>
      CapturedDisplay(
        path: value['path'] as String,
        x: (value['x'] as num).toDouble(),
        y: (value['y'] as num).toDouble(),
        width: (value['width'] as num).toDouble(),
        height: (value['height'] as num).toDouble(),
        pixelWidth: value['pixelWidth'] as int,
        pixelHeight: value['pixelHeight'] as int,
        pixelX:
            (value['pixelX'] as num?)?.toInt() ??
            ((value['x'] as num) * (value['scale'] as num)).round(),
        pixelY:
            (value['pixelY'] as num?)?.toInt() ??
            ((value['y'] as num) * (value['scale'] as num)).round(),
        scale: (value['scale'] as num).toDouble(),
      );

  Map<String, dynamic> toMap() => {
    'path': path,
    'x': x,
    'y': y,
    'width': width,
    'height': height,
    'pixelWidth': pixelWidth,
    'pixelHeight': pixelHeight,
    'pixelX': pixelX,
    'pixelY': pixelY,
    'scale': scale,
  };
}

class CaptureBridge {
  static const _channel = MethodChannel('snappin/capture');

  static Map<String, Object> longCaptureRegion(
    CapturedDisplay display,
    Rect selection, {
    bool? macOS,
  }) {
    final logicalX = display.x + selection.left;
    final logicalY = display.y + selection.top;
    final useLogicalPoints = macOS ?? Platform.isMacOS;
    final x = useLogicalPoints
        ? logicalX.round()
        : display.pixelX + (selection.left * display.scale).round();
    final y = useLogicalPoints
        ? logicalY.round()
        : display.pixelY + (selection.top * display.scale).round();
    final width = (selection.width * (useLogicalPoints ? 1 : display.scale))
        .round();
    final height = (selection.height * (useLogicalPoints ? 1 : display.scale))
        .round();
    return {
      'x': x + 4,
      'y': y + 4,
      'width': width - 8,
      'height': height - 8,
      'logicalX': logicalX,
      'logicalY': logicalY,
      'logicalWidth': selection.width,
      'logicalHeight': selection.height,
    };
  }

  static Future<Rect?> windowSnap(
    CapturedDisplay display,
    double x,
    double y,
  ) async {
    final result = await _channel.invokeMapMethod<String, dynamic>(
      'windowSnap',
      {
        'x': display.x + x,
        'y': display.y + y,
        'pixelX': display.pixelX + (x * display.scale).round(),
        'pixelY': display.pixelY + (y * display.scale).round(),
        'displayX': display.x,
        'displayY': display.y,
        'displayPixelX': display.pixelX,
        'displayPixelY': display.pixelY,
        'displayWidth': display.width.round(),
        'displayHeight': display.height.round(),
        'scale': display.scale,
      },
    );
    if (result == null) return null;
    return Rect.fromLTWH(
      (result['x'] as num).toDouble(),
      (result['y'] as num).toDouble(),
      (result['width'] as num).toDouble(),
      (result['height'] as num).toDouble(),
    );
  }

  static void onLongEscape(Future<void> Function() callback) {
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'longEscape') await callback();
    });
  }

  static Future<void> setLongEscapeEnabled(bool enabled) =>
      _channel.invokeMethod<void>('setLongEscape', enabled);

  static Future<void> scrollAt(
    int x,
    int y, {
    int lines = 3,
    double? logicalX,
    double? logicalY,
  }) => _channel.invokeMethod<void>('scrollAt', {
    'x': x,
    'y': y,
    'lines': lines,
    if (logicalX != null) 'logicalX': logicalX,
    if (logicalY != null) 'logicalY': logicalY,
  });

  static Future<List<CapturedDisplay>> captureDisplays() async {
    final result = await _channel.invokeListMethod<dynamic>('captureDisplays');
    if (result == null || result.isEmpty) {
      throw StateError('No displays captured');
    }
    return result
        .map(
          (item) =>
              CapturedDisplay.fromMap(Map<String, dynamic>.from(item as Map)),
        )
        .toList();
  }

  static Future<Uint8List> captureRegion(
    int x,
    int y,
    int width,
    int height,
  ) async {
    final bytes = await _channel.invokeMethod<Uint8List>('captureRegion', {
      'x': x,
      'y': y,
      'width': width,
      'height': height,
    });
    if (bytes == null || bytes.isEmpty) throw StateError('选区采集失败');
    return bytes;
  }

  static Future<int> suppressInputToolbar(bool enabled) async {
    return await _channel.invokeMethod<int>('suppressInputToolbar', enabled) ??
        0;
  }
}
