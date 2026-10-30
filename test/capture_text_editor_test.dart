import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/capture_bridge.dart';
import 'package:snappin_flutter/windows/capture_overlay.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('text edits in place and Escape only cancels the editor', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1000, 700);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    const multiWindow = MethodChannel('mixin.one/desktop_multi_window');
    const capture = MethodChannel('snappin/capture');
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(multiWindow, (call) async {
      if (call.method == 'getAllWindows') return <Object>[];
      if (call.method == 'getWindowDefinition') {
        return {'windowId': 'test-overlay', 'windowArgument': ''};
      }
      return null;
    });
    messenger.setMockMethodCallHandler(capture, (call) async => null);
    addTearDown(() {
      messenger.setMockMethodCallHandler(multiWindow, null);
      messenger.setMockMethodCallHandler(capture, null);
    });

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: CaptureOverlay(
            controller: WindowController.fromWindowId('test-overlay'),
            display: CapturedDisplay(
              path: 'assets/tray_icon.png',
              x: 0,
              y: 0,
              width: 1000,
              height: 700,
              pixelWidth: 1000,
              pixelHeight: 700,
              scale: 1,
              pixelX: 0,
              pixelY: 0,
            ),
          ),
        ),
      ),
    );
    await tester.pump(const Duration(milliseconds: 200));

    await tester.tapAt(const Offset(500, 500));
    await tester.pump();
    expect(find.byTooltip('文字'), findsNothing);

    await tester.dragFrom(const Offset(150, 180), const Offset(350, 220));
    await tester.pump();
    await tester.tap(find.byTooltip('文字'));
    await tester.pump();
    await tester.tapAt(const Offset(200, 220));
    await tester.pump();

    expect(find.byType(TextField), findsOneWidget);
    await tester.enterText(find.byType(TextField), '测试文字');
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.pump();
    expect(find.byType(TextField), findsOneWidget);
    expect(find.byTooltip('文字'), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pump();
    expect(find.byType(TextField), findsNothing);
    expect(find.byTooltip('文字'), findsOneWidget);

    await tester.tapAt(const Offset(220, 240));
    await tester.pump();
    expect(find.byType(TextField), findsOneWidget);
    await tester.enterText(find.byType(TextField), '保留文字');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.enter);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump();
    expect(find.byType(TextField), findsNothing);
    expect(find.byTooltip('文字'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    imageCache.clear();
    imageCache.clearLiveImages();
  });
}
