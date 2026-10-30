import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/windows/pin_window.dart';
import 'package:snappin_flutter/windows/image_pixels.dart';
import 'package:snappin_flutter/windows/viewer_sticker_toolbar.dart';

void main() {
  testWidgets('pin viewer switches between original, fit, and edit modes', (
    tester,
  ) async {
    const manager = MethodChannel('window_manager');
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('mixin.one/desktop_multi_window/channels'),
          (call) async => null,
        );
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(manager, (call) async => null);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('mixin.one/desktop_multi_window'),
          (call) async => call.method == 'getWindowDefinition'
              ? {'windowId': 'pin-viewer-test', 'windowArgument': ''}
              : null,
        );
    final directory = Directory.systemTemp.createTempSync(
      'snappin-pin-viewer-',
    );
    final file = File('${directory.path}/pin.png');
    file.writeAsBytesSync(img.encodePng(img.Image(width: 160, height: 600)));
    try {
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: PinWindow(
              controller: WindowController.fromWindowId('pin-viewer-test'),
              path: file.path,
              baseSize: const Size(160, 600),
            ),
          ),
        ),
      );
      expect(find.byTooltip('图片适应窗口大小'), findsOneWidget);
      expect(find.byTooltip('文字识别'), findsOneWidget);
      final more = tester.widget<PopupMenuButton<String>>(
        find.byType(PopupMenuButton<String>),
      );
      expect(more.constraints, const BoxConstraints.tightFor(width: 160));
      await tester.tap(find.byTooltip('更多'));
      await tester.pumpAndSettle();
      final open = tester.widget<Text>(find.text('使用系统默认方式打开'));
      expect(open.style?.fontSize, 14);
      expect(
        tester
            .widget<PopupMenuItem<String>>(
              find.ancestor(
                of: find.text('使用系统默认方式打开'),
                matching: find.byType(PopupMenuItem<String>),
              ),
            )
            .padding,
        const EdgeInsets.symmetric(horizontal: 10),
      );
      await tester.tapAt(const Offset(750, 550));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('图片适应窗口大小'));
      await tester.pump();
      expect(find.byTooltip('图片原始大小'), findsOneWidget);
      await tester.tap(find.byTooltip('编辑'));
      await tester.pump();
      expect(find.text('完成'), findsOneWidget);
      expect(find.byTooltip('矩形'), findsOneWidget);
      await tester.tap(find.byTooltip('贴纸'));
      await tester.pumpAndSettle();
      expect(find.byType(ViewerStickerToolbar), findsOneWidget);
      expect(find.text('🚀'), findsOneWidget);
      await tester.tap(find.text('🚀'));
      await tester.tapAt(tester.getCenter(find.byType(ImagePixels)));
      await tester.pump();
      await tester.tap(find.byTooltip('文字'));
      await tester.pump();
      await tester.tapAt(
        tester.getCenter(find.byType(ImagePixels)) + const Offset(0, 40),
      );
      await tester.pump();
      expect(find.byType(AlertDialog), findsNothing);
      expect(find.byType(TextField), findsOneWidget);
      await tester.enterText(find.byType(TextField), '行内输入');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pump();
      expect(find.byType(TextField), findsOneWidget);
      await tester.tap(find.byTooltip('关闭'));
      await tester.pumpAndSettle();
      expect(find.text('有未保存的修改'), findsOneWidget);
      await tester.tap(find.text('继续编辑'));
      await tester.pumpAndSettle();
      expect(find.text('完成'), findsOneWidget);
      await tester.tap(find.text('完成'));
      await tester.pump();
      expect(find.text('保存修改中'), findsOneWidget);
      var saved = false;
      for (var i = 0; i < 150; i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
        saved = directory.listSync().any(
          (file) => file.path.contains('viewer-edit-'),
        );
        if (saved && find.text('保存修改中').evaluate().isEmpty) break;
      }
      expect(saved, isTrue, reason: 'Edited image was not saved');
      await tester.pumpAndSettle();
      expect(find.text('保存修改中'), findsNothing);
      expect(find.text('修改已保存'), findsOneWidget);
      expect(tester.takeException(), isNull);
    } finally {
      await tester.pumpWidget(const SizedBox.shrink());
      await FileImage(file).evict();
      PaintingBinding.instance.imageCache.clear();
      PaintingBinding.instance.imageCache.clearLiveImages();
      await tester.pumpAndSettle();
      try {
        directory.deleteSync(recursive: true);
      } on FileSystemException {
        // Windows may retain a short lived image decoder handle until the test process exits.
      }
    }
  });
}
