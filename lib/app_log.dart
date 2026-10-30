import 'dart:io';

import 'package:flutter/foundation.dart';

class AppLog {
  AppLog._();

  static File? _file;

  static String get path => _file?.path ?? '';

  static void initialize() {
    final base = Platform.isMacOS
        ? '${Platform.environment['HOME'] ?? Directory.systemTemp.path}/Library/Application Support'
        : Platform.environment['APPDATA'] ?? Directory.systemTemp.path;
    final directory = Directory(
      '$base${Platform.pathSeparator}com.snappin.flutter'
      '${Platform.pathSeparator}logs',
    );
    try {
      directory.createSync(recursive: true);
      final now = DateTime.now();
      final day = '${now.year}-${now.month.toString().padLeft(2, '0')}-${now.day.toString().padLeft(2, '0')}';
      _file = File('${directory.path}${Platform.pathSeparator}snappin-$day.log');
      for (final entry in directory.listSync().whereType<File>()) {
        if (entry.path.endsWith('.log') && now.difference(entry.lastModifiedSync()).inDays > 7) {
          entry.deleteSync();
        }
      }
    } catch (error) {
      debugPrint('Log initialization failed: $error');
    }
  }

  static void write(String event, [Object? detail]) {
    final line = '${DateTime.now().toIso8601String()} [$event]${detail == null ? '' : ' $detail'}';
    debugPrint(line);
    try {
      _file?.writeAsStringSync('$line\n', mode: FileMode.append, flush: true);
    } catch (error) {
      debugPrint('Log write failed: $error');
    }
  }
}
