import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hotkey_manager/hotkey_manager.dart';
import 'package:snappin_flutter/shortcuts.dart';

void main() {
  test('default shortcuts use release-safe logical keys', () {
    final capture = parseShortcut('Ctrl+1');
    final pin = parseShortcut('Ctrl+2');
    expect(capture.key, LogicalKeyboardKey.digit1);
    expect(pin.key, LogicalKeyboardKey.digit2);
    expect(capture.modifiers, contains(HotKeyModifier.control));
    expect(pin.modifiers, contains(HotKeyModifier.control));
  });
}
