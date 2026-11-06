import 'dart:async';
import 'dart:convert';

import 'package:desktop_multi_window/desktop_multi_window.dart';

/// Retains the full-resolution result across creation/engine-readiness failures.
/// A retry reuses a live engine instead of creating duplicate result windows.
class LongResultDelivery {
  Map<String, dynamic>? pending;
  WindowController? _window;

  Future<void> open({
    Duration readyTimeout = const Duration(seconds: 4),
  }) async {
    final output = pending;
    if (output == null) return;
    if (_window != null &&
        !(await WindowController.getAll()).any(
          (w) => w.windowId == _window!.windowId,
        )) {
      _window = null;
    }
    _window ??= await WindowController.create(
      WindowConfiguration(arguments: jsonEncode(output), hiddenAtLaunch: true),
    );
    final elapsed = Stopwatch()..start();
    while (elapsed.elapsed < readyTimeout) {
      try {
        final path = await _window!
            .invokeMethod<String>('pinReady')
            .timeout(const Duration(milliseconds: 200));
        if (path == output['path']) {
          pending = null;
          _window = null;
          return;
        }
      } catch (_) {
        // Method registration and source loading can lag native window creation.
      }
      await Future<void>.delayed(const Duration(milliseconds: 50));
    }
    throw StateError('长图结果窗口尚未就绪，原图已保留，可重试');
  }

  void abandon() {
    pending = null;
    _window = null;
  }
}
