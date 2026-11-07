import 'dart:io';
import 'dart:convert';
import 'dart:ui' as ui;

import 'package:flutter/rendering.dart';

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
    final nativeCalls = <MethodCall>[];
    final boundary = GlobalKey();
    final entries = <Map<String, dynamic>>[];
    Future<void> snapshot(String name) async {
      final review = Platform.environment['SNAPPIN_VISUAL_REVIEW'];
      if (review == null) return;
      await tester.runAsync(() async {
        final image =
            await (boundary.currentContext!.findRenderObject()
                    as RenderRepaintBoundary)
                .toImage();
        final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
        await File('$review/$name.png')
            .writeAsBytes(bytes!.buffer.asUint8List());
        image.dispose();
      });
    }

    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('mixin.one/desktop_multi_window/channels'),
          (call) async =>
              call.method == 'invokeMethod' ? jsonEncode(entries) : null,
        );
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(manager, (call) async {
          nativeCalls.add(call);
          return null;
        });
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('mixin.one/desktop_multi_window'),
          (call) async => call.method == 'getWindowDefinition'
              ? {'windowId': 'pin-viewer-test', 'windowArgument': ''}
              : call.method == 'getAllWindows'
              ? [
                  {
                    'windowId': 'history-ball',
                    'windowArgument': '{"role":"ball"}',
                  },
                ]
              : null,
        );
    final directory = Directory.systemTemp.createTempSync(
      'snappin-pin-viewer-',
    );
    final file = File('${directory.path}/pin.png');
    file.writeAsBytesSync(img.encodePng(img.Image(width: 160, height: 600)));
    final secondFile = File('${directory.path}/second.png')
      ..writeAsBytesSync(img.encodePng(img.Image(width: 100, height: 400)));
    entries.add({
      'path': secondFile.path,
      'day': '2026-11-08',
      'name': 'second.png',
      'width': 100,
      'height': 400,
    });
    entries.add({
      'path': file.path,
      'day': '2026-11-07',
      'name': 'original.png',
      'width': 160,
      'height': 600,
    });
    try {
      await tester.pumpWidget(
        MaterialApp(
          builder: (context, child) =>
              RepaintBoundary(key: boundary, child: child!),
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
      // Reference screenshot coordinates include the one-pixel outer border.
      for (final item in [
        ('窗口置顶', 26.0),
        ('上一张', 78.0),
        ('下一张', 118.0),
        ('截图历史', 158.0),
        ('放大', 212.0),
        ('缩小', 252.0),
        ('向右旋转', 344.0),
        ('编辑', 384.0),
        ('文字识别', 438.0),
        ('全屏', 478.0),
        ('保存', 518.0),
      ]) {
        final center = tester.getCenter(find.byTooltip(item.$1));
        expect(center.dx, item.$2, reason: item.$1);
        expect(center.dy, 20.5, reason: item.$1);
      }

      await tester.pumpAndSettle();
      await snapshot('贴图结果页');
      await tester.tap(find.byTooltip('截图历史'));
      await tester.pumpAndSettle();
      expect(find.text('2026-11-07'), findsOneWidget);
      await snapshot('贴图结果页-历史栏');
      await tester.tap(
        find.byKey(ValueKey('archive-thumbnail-${secondFile.path}')),
      );
      for (var i = 0; i < 30; i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
      await tester.pumpAndSettle();
      expect(
        tester.widget<ImagePixels>(find.byType(ImagePixels)).path,
        secondFile.path,
      );
      await tester.tap(find.byKey(ValueKey('archive-thumbnail-${file.path}')));
      for (var i = 0; i < 30; i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
      await tester.pumpAndSettle();
      expect(
        tester.widget<ImagePixels>(find.byType(ImagePixels)).path,
        file.path,
      );
      expect(secondFile.existsSync(), isTrue);
      await tester.tap(find.byTooltip('截图历史'));
      await tester.pumpAndSettle();
      expect(find.text('2026-11-07'), findsNothing);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyY);
      await tester.pump();
      expect(
        nativeCalls
            .lastWhere((call) => call.method == 'setHasShadow')
            .arguments,
        {'hasShadow': false},
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.keyY);
      await tester.pump();
      expect(
        nativeCalls
            .lastWhere((call) => call.method == 'setHasShadow')
            .arguments,
        {'hasShadow': true},
      );
      final more = tester.widget<PopupMenuButton<String>>(
        find.byType(PopupMenuButton<String>),
      );
      expect(
        more.constraints,
        const BoxConstraints(minWidth: 160, maxWidth: 224),
      );
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
        final result = tester.widget<ImagePixels>(find.byType(ImagePixels));
        saved = result.path != file.path && File(result.path).existsSync();
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
