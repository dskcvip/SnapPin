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
  final label = event.logicalKey.keyLabel;
  if (label.isEmpty ||
      label.toLowerCase().contains('control') ||
      label.toLowerCase().contains('shift') ||
      label.toLowerCase().contains('alt') ||
      label.toLowerCase().contains('meta')) {
    return null;
  }
  return label.toUpperCase();
}
