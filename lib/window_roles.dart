import 'dart:convert';

import 'package:desktop_multi_window/desktop_multi_window.dart';

String windowRole(WindowController window) {
  if (window.arguments.isEmpty) return 'ball';
  try {
    final value = jsonDecode(window.arguments);
    if (value is Map) return value['role'] as String? ?? 'ball';
  } catch (_) {}
  return 'unknown';
}

Future<WindowController?> findBallWindow() async {
  for (final window in await WindowController.getAll()) {
    if (windowRole(window) == 'ball') return window;
  }
  return null;
}
