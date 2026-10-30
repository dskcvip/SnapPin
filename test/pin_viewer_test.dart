import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/windows/pin_window.dart';

void main() {
  testWidgets('pin viewer switches between original, fit, and edit modes', (
    tester,
  ) async {
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
