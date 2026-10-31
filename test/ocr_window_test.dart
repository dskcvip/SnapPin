import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/windows/ocr_window.dart';

void main() {
  testWidgets('OCR pin button starts selected and toggles the native window', (
    tester,
  ) async {
    final calls = <MethodCall>[];
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(const MethodChannel('window_manager'), (
      call,
    ) async {
      calls.add(call);
      return null;
    });
    messenger.setMockMethodCallHandler(
      const MethodChannel('mixin.one/desktop_multi_window/channels'),
      (_) async => null,
    );
    final directory = Directory.systemTemp.createTempSync(
      'snappin-ocr-ui-test-',
    );
    final path = '${directory.path}/source.png';
    File(path).writeAsBytesSync(img.encodePng(img.Image(width: 1, height: 1)));
    addTearDown(() async {
      await tester.pumpWidget(const SizedBox.shrink());
      messenger.setMockMethodCallHandler(
        const MethodChannel('window_manager'),
        null,
      );
      messenger.setMockMethodCallHandler(
        const MethodChannel('mixin.one/desktop_multi_window/channels'),
        null,
      );
      directory.deleteSync(recursive: true);
    });
    await tester.pumpWidget(
      MaterialApp(
        home: OcrWindow(
          controller: WindowController.fromWindowId('ocr-test'),
          imagePath: path,
          text: '识别结果',
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byIcon(Icons.push_pin), findsOneWidget);
    await tester.tap(find.byTooltip('取消置顶'));
    await tester.pumpAndSettle();
    expect(find.byIcon(Icons.push_pin_outlined), findsOneWidget);
    expect(
      calls.lastWhere((call) => call.method == 'setAlwaysOnTop').arguments,
      {'isAlwaysOnTop': false},
    );
    await tester.tap(find.byTooltip('窗口置顶'));
    await tester.pumpAndSettle();
    expect(find.byIcon(Icons.push_pin), findsOneWidget);
    expect(
      calls.lastWhere((call) => call.method == 'setAlwaysOnTop').arguments,
      {'isAlwaysOnTop': true},
    );
  });
}
