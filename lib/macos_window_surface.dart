import 'dart:io';

import 'package:flutter/services.dart';

import 'app_log.dart';

class MacWindowSurface {
  static const _channel = MethodChannel('snappin/capture');

  static Future<Map<String, dynamic>?> configure({Rect? overlayBounds}) async {
    if (!Platform.isMacOS) return null;
    final status = await _channel.invokeMapMethod<String, dynamic>(
      'configureWindowSurface',
      overlayBounds == null
          ? null
          : {
              'x': overlayBounds.left,
              'y': overlayBounds.top,
              'width': overlayBounds.width,
              'height': overlayBounds.height,
            },
    );
    if (status?['ready'] != true ||
        status?['opaque'] != false ||
        status?['windowAlpha'] != 0 ||
        status?['flutterAlpha'] != 0) {
      throw StateError('macOS 窗口透明配置失败：$status');
    }
    if (overlayBounds != null) {
      final expected = {
        'x': overlayBounds.left,
        'y': overlayBounds.top,
        'width': overlayBounds.width,
        'height': overlayBounds.height,
        'contentWidth': overlayBounds.width,
        'contentHeight': overlayBounds.height,
        'viewWidth': overlayBounds.width,
        'viewHeight': overlayBounds.height,
      };
      for (final item in expected.entries) {
        final actual = status?[item.key] as num?;
        if (actual == null || (actual - item.value).abs() > .5) {
          throw StateError('macOS 截图窗口尺寸不一致：$status expected=$expected');
        }
      }
    }
    AppLog.write('macos-window-surface-ready', status);
    return status;
  }

  static Future<void> verifyTransparentSmoke(String role) async {
    final status = await inspect();
    if (status?['opaque'] != false ||
        status?['windowAlpha'] != 0 ||
        status?['flutterAlpha'] != 0 ||
        status?['cornerAlpha'] != 0) {
      throw StateError('$role has an opaque surface or corner: $status');
    }
    AppLog.write('macos-transparent-smoke-ready', '$role $status');
  }

  static Future<Map<String, dynamic>?> inspect() =>
      _channel.invokeMapMethod<String, dynamic>('inspectWindowSurface');
}
