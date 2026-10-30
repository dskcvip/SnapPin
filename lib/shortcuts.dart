import 'package:flutter/services.dart';
import 'package:hotkey_manager/hotkey_manager.dart';

HotKey parseShortcut(String spec) {
  final parts = spec
      .split('+')
      .map((part) => part.trim().toLowerCase())
      .toList();
  if (parts.isEmpty) throw FormatException('快捷键为空');
  final name = parts.last;
  final digit = int.tryParse(name);
  final key = digit != null && digit >= 0 && digit <= 9
      ? [
          LogicalKeyboardKey.digit0,
          LogicalKeyboardKey.digit1,
          LogicalKeyboardKey.digit2,
          LogicalKeyboardKey.digit3,
          LogicalKeyboardKey.digit4,
          LogicalKeyboardKey.digit5,
          LogicalKeyboardKey.digit6,
          LogicalKeyboardKey.digit7,
          LogicalKeyboardKey.digit8,
          LogicalKeyboardKey.digit9,
        ][digit]
      : LogicalKeyboardKey.knownLogicalKeys
            .where((candidate) => candidate.keyLabel.toLowerCase() == name)
            .firstOrNull;
  if (key == null) throw FormatException('不支持的快捷键：$spec');
  return HotKey(
    key: key,
    modifiers: [
      if (parts.contains('ctrl') || parts.contains('control'))
        HotKeyModifier.control,
      if (parts.contains('super') ||
          parts.contains('meta') ||
          parts.contains('cmd'))
        HotKeyModifier.meta,
      if (parts.contains('alt')) HotKeyModifier.alt,
      if (parts.contains('shift')) HotKeyModifier.shift,
    ],
  );
}

String? shortcutKeyLabel(KeyDownEvent event) {
  // Match Tauri's KeyboardEvent.code: Shift+1 must remain "1", not "!".
  final usage = event.physicalKey.usbHidUsage;
  if (usage >= 0x70004 && usage <= 0x7001d) {
    return String.fromCharCode('A'.codeUnitAt(0) + usage - 0x70004);
  }
  if (usage >= 0x7001e && usage <= 0x70026) {
    return '${usage - 0x7001e + 1}';
  }
  if (usage == 0x70027) return '0';
  if (usage >= 0x7003a && usage <= 0x70045) {
    return 'F${usage - 0x7003a + 1}';
  }
  return null;
}
