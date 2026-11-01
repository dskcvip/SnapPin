import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/windows/app_menu_metrics.dart';
import 'package:snappin_flutter/windows/app_menu_window.dart';

void main() {
  for (final platform in [TargetPlatform.windows, TargetPlatform.macOS]) {
    for (final warnings in <List<String>>[
      [],
      ['截图快捷键'],
      ['贴图快捷键'],
      ['截图快捷键', '贴图快捷键'],
    ]) {
      testWidgets(
        '$platform app menu displays complete warning for $warnings',
        (tester) async {
          tester.view.physicalSize = Size(
            appMenuWidth,
            appMenuHeight(warnings, macOS: platform == TargetPlatform.macOS),
          );
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
              theme: ThemeData(platform: platform),
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
          expect(
            find.text('权限检查…'),
            platform == TargetPlatform.macOS ? findsOneWidget : findsNothing,
          );
          expect(
            tester.getBottomRight(find.text('退出')).dy,
            lessThan(tester.view.physicalSize.height),
          );
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
        },
      );
    }
  }

  testWidgets('macOS permission menu dispatches to the main engine', (
    tester,
  ) async {
    tester.view.physicalSize = Size(
      appMenuWidth,
      appMenuHeight([], macOS: true),
    );
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    final actions = <Object?>[];
    const windows = MethodChannel('mixin.one/desktop_multi_window');
    const channels = MethodChannel('mixin.one/desktop_multi_window/channels');
    messenger.setMockMethodCallHandler(windows, (call) async {
      if (call.method == 'getAllWindows') {
        return [
          {'windowId': 'ball-test', 'windowArgument': ''},
        ];
      }
      if (call.method == 'getWindowDefinition') {
        return {'windowId': 'permission-menu-test', 'windowArgument': ''};
      }
      return null;
    });
    messenger.setMockMethodCallHandler(channels, (call) async {
      if (call.method == 'invokeMethod') {
        final args = call.arguments as Map;
        if (args['method'] == 'menuAction') actions.add(args['arguments']);
      }
      return null;
    });
    messenger.setMockMethodCallHandler(
      const MethodChannel('window_manager'),
      (_) async => null,
    );
    addTearDown(() {
      messenger.setMockMethodCallHandler(windows, null);
      messenger.setMockMethodCallHandler(channels, null);
      messenger.setMockMethodCallHandler(
        const MethodChannel('window_manager'),
        null,
      );
    });
    await tester.pumpWidget(
      MaterialApp(
        theme: ThemeData(platform: TargetPlatform.macOS),
        home: Scaffold(
          body: AppMenuWindow(
            controller: WindowController.fromWindowId('permission-menu-test'),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('权限检查…'));
    await tester.pumpAndSettle();
    expect(actions, ['perm-check']);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
  });
}
