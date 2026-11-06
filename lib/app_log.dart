import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';

class AppLog {
  AppLog._();

  static File? _file;
  static final _pending = StringBuffer();
  static Timer? _timer;

  static String get path => _file?.path ?? '';

  static void initialize({Directory? logDirectory}) {
    flush();
    final base = Platform.isMacOS
        ? '${Platform.environment['HOME'] ?? Directory.systemTemp.path}/Library/Application Support'
        : Platform.environment['APPDATA'] ?? Directory.systemTemp.path;
    final directory =
        logDirectory ??
        Directory(
          '$base${Platform.pathSeparator}com.snappin.flutter'
          '${Platform.pathSeparator}logs',
        );
    try {
      directory.createSync(recursive: true);
      final now = DateTime.now();
      final day =
          '${now.year}-${now.month.toString().padLeft(2, '0')}-${now.day.toString().padLeft(2, '0')}';
      _file = File(
        '${directory.path}${Platform.pathSeparator}snappin-$day.log',
      );
    } catch (error) {
      debugPrint('Log initialization failed: $error');
    }
  }

  static void write(String event, [Object? detail]) {
    final line =
        '${DateTime.now().toIso8601String()} [$event]${detail == null ? '' : ' $detail'}';
    debugPrint(line);
    if (_file == null) return;
    _pending.writeln(line);
    // Bound the buffer without making every sampled frame wait for fsync.
    if (_pending.length >= 64 * 1024 ||
        event.contains('failed') ||
        event.contains('error') ||
        event.contains('quit') ||
        event.contains('finished') ||
        event.contains('ready') ||
        event.contains('closed')) {
      flush();
    } else {
      _timer ??= Timer(const Duration(milliseconds: 100), flush);
    }
  }

  /// Call before terminating an engine/process or opening the log for review.
  static void flush() {
    _timer?.cancel();
    _timer = null;
    if (_pending.isEmpty) return;
    final lines = _pending.toString();
    _pending.clear();
    try {
      _file?.writeAsStringSync(lines, mode: FileMode.append, flush: true);
    } catch (error) {
      debugPrint('Log write failed: $error');
    }
  }
}
