import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/app_config.dart';
import 'package:snappin_flutter/shortcuts.dart';
import 'package:snappin_flutter/windows/settings_window.dart';

void main() {
  test('shortcut recording uses physical keys for shifted digits', () {
    expect(
      shortcutKeyLabel(
        const KeyDownEvent(
          physicalKey: PhysicalKeyboardKey.digit1,
          logicalKey: LogicalKeyboardKey.exclamation,
          timeStamp: Duration.zero,
        ),
      ),
      '1',
    );
    expect(
      shortcutKeyLabel(
        const KeyDownEvent(
          physicalKey: PhysicalKeyboardKey.controlLeft,
          logicalKey: LogicalKeyboardKey.controlLeft,
          timeStamp: Duration.zero,
        ),
      ),
      isNull,
    );
  });

  testWidgets(
    'recording suspends global shortcuts and restores them on input, Escape and navigation',
    (tester) async {
      tester.view.physicalSize = const Size(782, 640);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final calls = <String>[];
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(
        const MethodChannel('mixin.one/desktop_multi_window'),
        (call) async {
          if (call.method == 'getWindowDefinition')
            return {'windowId': 'settings-recording', 'windowArgument': ''};
          if (call.method == 'getAllWindows')
            return [
              {
                'windowId': 'recording-ball',
                'windowArgument': '{"role":"ball"}',
              },
            ];
          return null;
        },
      );
      messenger.setMockMethodCallHandler(
        const MethodChannel('mixin.one/desktop_multi_window/channels'),
        (call) async {
          if (call.method == 'invokeMethod')
            calls.add((call.arguments as Map)['method'] as String);
          return null;
        },
      );
      messenger.setMockMethodCallHandler(
        const MethodChannel('window_manager'),
        (_) async => null,
      );
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: SettingsWindow(
              controller: WindowController.fromWindowId('settings-recording'),
              initialConfig: const AppConfig(),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('快捷键').first);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Ctrl+1'));
      await tester.pumpAndSettle();
      expect(calls.last, 'suspendHotkeys');
      expect(find.text('按下新组合键…'), findsOneWidget);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(
        LogicalKeyboardKey.digit1,
        physicalKey: PhysicalKeyboardKey.digit1,
      );
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pumpAndSettle();
      expect(calls.last, 'resumeHotkeys');
      expect(find.text('Ctrl+Shift+1'), findsOneWidget);

      await tester.tap(find.text('Ctrl+2'));
      await tester.pumpAndSettle();
      expect(calls.last, 'suspendHotkeys');
      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(calls.last, 'resumeHotkeys');
      expect(find.text('有未保存的修改'), findsNothing);

      await tester.tap(find.text('Ctrl+2'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('通用').first);
      await tester.pumpAndSettle();
      expect(calls.last, 'resumeHotkeys');
      expect(calls.where((method) => method == 'suspendHotkeys').length, 3);
      expect(calls.where((method) => method == 'resumeHotkeys').length, 3);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
}
