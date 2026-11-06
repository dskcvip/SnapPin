import 'dart:convert';
import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/app_config.dart';
import 'package:snappin_flutter/windows/screenshot_history.dart';

void main() {
  test(
    'archive scope defaults and explicit empty scope survive config round trip',
    () {
      expect(AppConfig.fromJson({}).archiveSources, [
        'capture',
        'pin',
        'longCapture',
      ]);
      final config = const AppConfig().copyWith(archiveSources: []);
      expect(AppConfig.fromJson(config.toJson()).archiveSources, isEmpty);
      expect(
        AppConfig.fromJson({
          'archiveSources': ['ocr', 'unknown'],
        }).archiveSources,
        ['ocr'],
      );
    },
  );

  testWidgets(
    'history groups dates, changes date order, previews on the left and selects originals',
    (tester) async {
      final directory = Directory.systemTemp.createTempSync('snappin-history-');
      final file = File('${directory.path}/original.png')
        ..writeAsBytesSync(img.encodePng(img.Image(width: 180, height: 300)));
      final entries = [
        {
          'path': file.path,
          'day': '2026-11-07',
          'name': 'new.png',
          'width': 180,
          'height': 300,
        },
        {
          'path': file.path,
          'day': '2026-11-06',
          'name': 'old.png',
          'width': 180,
          'height': 300,
        },
      ];
      final messenger =
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(
        const MethodChannel('mixin.one/desktop_multi_window'),
        (call) async => call.method == 'getAllWindows'
            ? [
                {
                  'windowId': 'history-ball',
                  'windowArgument': '{"role":"ball"}',
                },
              ]
            : null,
      );
      messenger.setMockMethodCallHandler(
        const MethodChannel('mixin.one/desktop_multi_window/channels'),
        (call) async =>
            call.method == 'invokeMethod' ? jsonEncode(entries) : null,
      );
      Map<String, dynamic>? selected;
      try {
        await tester.pumpWidget(
          MaterialApp(
            home: Scaffold(
              body: Row(
                children: [
                  const Expanded(child: SizedBox()),
                  ScreenshotHistory(
                    currentPath: file.path,
                    onSelected: (entry) async {
                      selected = entry;
                    },
                  ),
                ],
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        expect(
          tester.getTopLeft(find.text('2026-11-07')).dy,
          lessThan(tester.getTopLeft(find.text('2026-11-06')).dy),
        );
        final thumbnail = find.byType(Image).first;
        final mouse = await tester.createGesture(kind: PointerDeviceKind.mouse);
        await mouse.addPointer(location: Offset.zero);
        await mouse.moveTo(tester.getCenter(thumbnail));
        await tester.pump(const Duration(milliseconds: 250));
        await tester.pump();
        final preview = find.byKey(const ValueKey('screenshot-hover-preview'));
        expect(preview, findsOneWidget);
        expect(
          tester.getRect(preview).right,
          lessThan(tester.getRect(thumbnail).left),
        );
        await mouse.moveTo(const Offset(100, 500));
        await tester.pumpAndSettle();
        expect(preview, findsNothing);
        await tester.tap(thumbnail);
        await tester.pump();
        expect(selected?['name'], 'new.png');
        expect(
          file.readAsBytesSync(),
          img.encodePng(img.Image(width: 180, height: 300)),
        );
        await tester.tap(find.byTooltip('日期：从新到旧'));
        await tester.pumpAndSettle();
        expect(
          tester.getTopLeft(find.text('2026-11-06')).dy,
          lessThan(tester.getTopLeft(find.text('2026-11-07')).dy),
        );
        await mouse.removePointer();
        expect(tester.takeException(), isNull);
      } finally {
        await tester.pumpWidget(const SizedBox());
        PaintingBinding.instance.imageCache.clear();
        directory.deleteSync(recursive: true);
      }
    },
  );
}
