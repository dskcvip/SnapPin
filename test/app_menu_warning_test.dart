import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/windows/app_menu_metrics.dart';
import 'package:snappin_flutter/windows/app_menu_window.dart';

void main() {
  for (final warnings in <List<String>>[
    [],
    ['截图快捷键'],
    ['贴图快捷键'],
    ['截图快捷键', '贴图快捷键'],
  ]) {
    testWidgets('app menu displays complete warning for $warnings', (
      tester,
    ) async {
      tester.view.physicalSize = Size(appMenuWidth, appMenuHeight(warnings));
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      for (final channel in [
        'mixin.one/desktop_multi_window/channels',
        'window_manager',
      ]) {
        messenger.setMockMethodCallHandler(
          MethodChannel(channel),
          (_) async => null,
        );
      }
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: AppMenuWindow(
              controller: WindowController.fromWindowId('menu-test'),
              initialWarnings: warnings,
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('截图'), findsOneWidget);
      expect(find.text('退出'), findsOneWidget);
      if (warnings.isNotEmpty) {
        final warning = find.text(appMenuWarningText(warnings));
        expect(warning, findsOneWidget);
        expect(
          tester.widget<Text>(warning).overflow,
          isNot(TextOverflow.ellipsis),
        );
        expect(
          tester.getTopLeft(warning).dy,
          lessThan(tester.getTopLeft(find.text('截图')).dy),
        );
      } else {
        expect(find.textContaining('已被其他程序占用'), findsNothing);
      }
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
    });
  }
}
