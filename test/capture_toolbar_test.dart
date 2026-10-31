import 'dart:convert';
import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/capture_bridge.dart';
import 'package:snappin_flutter/ocr_image_store.dart';
import 'package:snappin_flutter/windows/capture_overlay.dart';
import 'package:zxing_lib/qrcode.dart';
import 'package:zxing_lib/zxing.dart' show BarcodeFormat;

const _ocr = '文字识别（识别当前选区内的文字，支持中文）';
const _translate = '翻译（框选后一键识别并翻译，左原文右译文）';
const _long = '长截图（滚动页面，实时预览；Esc 完成）';

class _Session {
  late Directory directory;
  final created = <Map<String, dynamic>>[];
  final dispatched = <Map<dynamic, dynamic>>[];
  final native = <MethodCall>[];
  Uint8List? copied;
  bool rejectOcr = false;
  bool cancelSave = false;
  bool includeQr = false;
  String get saved => '${directory.path}/saved.png';

  Future<void> open(WidgetTester tester) async {
    tester.view.physicalSize = const Size(1400, 900);
    tester.view.devicePixelRatio = 1;
    directory = Directory.systemTemp.createTempSync('snappin-toolbar-test-');
    final fixture = img.Image(width: 1400, height: 900);
    for (final pixel in fixture) {
      pixel.setRgb(pixel.x % 256, pixel.y % 256, (pixel.x + pixel.y) % 256);
    }
    if (includeQr) {
      final qr = QRCodeWriter().encode(
        'SnapPin-toolbar-qr-test',
        BarcodeFormat.qrCode,
        200,
        200,
      );
      for (var y = 0; y < 200; y++) {
        for (var x = 0; x < 200; x++) {
          final value = qr.get(x, y) ? 0 : 255;
          fixture.setPixelRgb(x + 600, y + 200, value, value, value);
        }
      }
    }
    final path = '${directory.path}/source.png';
    File(path).writeAsBytesSync(img.encodePng(fixture));
    final messenger = tester.binding.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(
      const MethodChannel('mixin.one/desktop_multi_window'),
      (call) async {
        if (call.method == 'getWindowDefinition') {
          return {'windowId': 'toolbar-overlay', 'windowArgument': ''};
        }
        if (call.method == 'getAllWindows') {
          return [
            {'windowId': 'ball-test', 'windowArgument': ''},
            {
              'windowId': 'toolbar-overlay',
              'windowArgument': '{"role":"overlay"}',
            },
          ];
        }
        if (call.method == 'createWindow') {
          created.add(
            Map<String, dynamic>.from(
              jsonDecode((call.arguments as Map)['arguments'] as String) as Map,
            ),
          );
          return 'created-${created.length}';
        }
        return null;
      },
    );
    messenger.setMockMethodCallHandler(
      const MethodChannel('mixin.one/desktop_multi_window/channels'),
      (call) async {
        if (call.method != 'invokeMethod') return null;
        final args = Map<dynamic, dynamic>.from(call.arguments as Map);
        dispatched.add(args);
        if (args['method'] == 'recognizeText') {
          if (rejectOcr) throw PlatformException(code: 'ocr_test_failure');
          final crop = img.decodePng(args['arguments'] as Uint8List)!;
          expect((crop.width, crop.height), (1000, 500));
          return '本地识别文字';
        }
        return null;
      },
    );
    for (final channel in ['window_manager', 'snappin/capture']) {
      messenger.setMockMethodCallHandler(MethodChannel(channel), (call) async {
        native.add(call);
        return null;
      });
    }
    messenger.setMockMethodCallHandler(const MethodChannel('pasteboard'), (
      call,
    ) async {
      if (call.method == 'writeImage') {
        copied = call.arguments is Uint8List
            ? call.arguments as Uint8List
            : File((call.arguments as Map)['fileName'] as String)
                  .readAsBytesSync();
      }
      return null;
    });
    messenger.setMockMethodCallHandler(
      const MethodChannel('plugins.flutter.io/file_selector'),
      (call) async {
        if (call.method == 'getSavePath') return cancelSave ? null : saved;
        return null;
      },
    );
    addTearDown(() async {
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      imageCache.clear();
      imageCache.clearLiveImages();
      for (final item in created) {
        if (item['role'] == 'ocr') {
          await OcrImageStore.deleteOwned(item['image'] as String);
        } else if (item['role'] == 'pin') {
          final file = File(item['path'] as String);
          if (file.existsSync()) file.deleteSync();
          if (file.parent.existsSync()) file.parent.deleteSync();
        }
      }
      for (final channel in [
        'mixin.one/desktop_multi_window',
        'mixin.one/desktop_multi_window/channels',
        'window_manager',
        'snappin/capture',
        'pasteboard',
        'plugins.flutter.io/file_selector',
      ]) {
        messenger.setMockMethodCallHandler(MethodChannel(channel), null);
      }
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
      try {
        directory.deleteSync(recursive: true);
      } on FileSystemException {
        /* Decoder may still own source. */
      }
    });
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: CaptureOverlay(
            controller: WindowController.fromWindowId('toolbar-overlay'),
            display: CapturedDisplay(
              path: path,
              x: 0,
              y: 0,
              width: 1400,
              height: 900,
              pixelWidth: 1400,
              pixelHeight: 900,
              scale: 1,
              pixelX: 0,
              pixelY: 0,
            ),
          ),
        ),
      ),
    );
    await settle(tester);
    await tester.dragFrom(const Offset(100, 100), const Offset(1000, 500));
    await tester.pumpAndSettle();
  }

  Future<void> settle(WidgetTester tester) async {
    for (var i = 0; i < 16; i++) {
      await tester.runAsync(
        () => Future<void>.delayed(const Duration(milliseconds: 20)),
      );
      await tester.pump(const Duration(milliseconds: 20));
    }
  }

  Future<void> click(WidgetTester tester, String tooltip) async {
    await tester.tap(find.byTooltip(tooltip));
    await settle(tester);
  }
}

void main() {
  for (final tool in [
    '矩形',
    '椭圆',
    '箭头',
    '画笔',
    '马赛克',
    '聚光灯',
    '序号',
    '贴纸',
    '文字',
    '水印',
    '智能擦除',
  ]) {
    testWidgets('toolbar $tool produces pixels in copied selection', (
      tester,
    ) async {
      final session = _Session();
      await session.open(tester);
      await session.click(tester, tool);
      await tester.dragFrom(const Offset(250, 220), const Offset(150, 90));
      await tester.pump();
      if (tool == '文字' || tool == '水印') {
        await tester.enterText(find.byType(TextField), '工具条验证');
        await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
        await tester.sendKeyEvent(LogicalKeyboardKey.enter);
        await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
        await tester.pump();
      }
      await session.click(tester, '完成并复制 (Enter)');
      expect(session.copied, isNotNull);
      final output = img.decodePng(session.copied!)!;
      expect((output.width, output.height), (1000, 500));
      var changed = 0;
      for (var y = 100; y < 250; y++) {
        for (var x = 100; x < 350; x++) {
          final pixel = output.getPixel(x, y);
          if (pixel.r != (x + 100) % 256 ||
              pixel.g != (y + 100) % 256 ||
              pixel.b != (x + y + 200) % 256) {
            changed++;
          }
        }
      }
      expect(
        changed,
        greaterThan(10),
        reason: '$tool did not affect exported pixels',
      );
      expect(
        session.created.any((item) => item['role'] == 'capture-toast'),
        isTrue,
      );
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets(
    'toolbar color, width and undo/redo affect actual exported pixels',
    (tester) async {
      final session = _Session();
      await session.open(tester);
      await session.click(tester, '矩形');
      await tester.tap(find.byTooltip('颜色').at(3));
      await tester.drag(find.byType(Slider), const Offset(45, 0));
      await tester.dragFrom(const Offset(250, 220), const Offset(150, 90));
      await tester.pump();
      await session.click(tester, '撤销 (Ctrl+Z)');
      final redo = find.descendant(
        of: find.byTooltip('重做 (Ctrl+Shift+Z / Ctrl+Y)'),
        matching: find.byType(InkWell),
      );
      expect(tester.widget<InkWell>(redo).onTap, isNotNull);
      await session.click(tester, '重做 (Ctrl+Shift+Z / Ctrl+Y)');
      await session.click(tester, '完成并复制 (Enter)');
      final output = img.decodePng(session.copied!)!;
      var green = 0;
      for (var y = 112; y < 128; y++) {
        final p = output.getPixel(220, y);
        if (p.r == 34 && p.g == 197 && p.b == 94) green++;
      }
      expect(green, greaterThan(2));
    },
  );

  for (final action in [
    _ocr,
    _translate,
    '贴图到屏幕',
    '保存到文件',
    _long,
    '取消截图 (Esc)',
  ]) {
    testWidgets('toolbar dispatches $action with full selection', (
      tester,
    ) async {
      final session = _Session();
      await session.open(tester);
      await session.click(tester, action);
      if (action == _ocr) {
        expect(session.created.single['role'], 'ocr');
        expect(session.created.single['text'], '本地识别文字');
        expect(
          File(session.created.single['image'] as String).existsSync(),
          isTrue,
        );
      } else if (action == _translate) {
        expect(session.created.single['role'], 'translation');
        expect(session.created.single['source'], '本地识别文字');
      } else if (action == '贴图到屏幕') {
        expect(session.created.single['role'], 'pin');
        final image = img.decodePng(
          File(session.created.single['path'] as String).readAsBytesSync(),
        )!;
        expect((image.width, image.height), (1000, 500));
      } else if (action == '保存到文件') {
        final image = img.decodePng(File(session.saved).readAsBytesSync())!;
        expect((image.width, image.height), (1000, 500));
      } else if (action == _long) {
        final start = session.dispatched.singleWhere(
          (call) => call['method'] == 'longCaptureStart',
        );
        final region = jsonDecode(start['arguments'] as String) as Map;
        expect(region['overlay'], 'toolbar-overlay');
        expect(region['logicalWidth'], 1000);
        expect(region['logicalHeight'], 500);
        // Sampling excludes the four-pixel selection border on each side.
        expect(region['width'], 992);
        expect(region['height'], 492);
        expect(
          session.native.any((call) => call.method == 'setIgnoreMouseEvents'),
          isTrue,
        );
      }
      if (action != _long) {
        expect(
          session.dispatched.any((call) => call['method'] == 'captureFinished'),
          isTrue,
        );
        expect(session.native.any((call) => call.method == 'hide'), isTrue);
      }
      expect(tester.takeException(), isNull);
    });
  }

  testWidgets('cancelled save and failed OCR preserve editable screenshot', (
    tester,
  ) async {
    final session = _Session()
      ..cancelSave = true
      ..rejectOcr = true;
    await session.open(tester);
    await session.click(tester, '保存到文件');
    expect(find.byTooltip('矩形'), findsOneWidget);
    expect(File(session.saved).existsSync(), isFalse);
    await session.click(tester, _ocr);
    expect(find.textContaining('识别失败：'), findsOneWidget);
    expect(find.byTooltip('矩形'), findsOneWidget);
    expect(
      session.dispatched.any((call) => call['method'] == 'captureFinished'),
      isFalse,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'toolbar eraser restores source pixels without erasing the source',
    (tester) async {
      final session = _Session();
      await session.open(tester);
      await session.click(tester, '矩形');
      await tester.dragFrom(const Offset(250, 220), const Offset(150, 90));
      await tester.pump();
      await session.click(tester, '橡皮擦');
      await tester.dragFrom(const Offset(250, 220), const Offset(150, 0));
      await tester.pump();
      await session.click(tester, '完成并复制 (Enter)');
      final output = img.decodePng(session.copied!)!;
      final erased = output.getPixel(220, 120);
      expect((erased.r, erased.g, erased.b), (64, 220, 28));
      final retained = output.getPixel(150, 170);
      expect((retained.r, retained.g, retained.b), (230, 45, 45));
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'toolbar QR selection decodes a real code and displays its contents',
    (tester) async {
      final session = _Session()..includeQr = true;
      await session.open(tester);
      await session.click(tester, '二维码识别（拖选区域，支持一图多码）');
      await tester.dragFrom(const Offset(580, 180), const Offset(240, 240));
      await session.settle(tester);
      for (
        var i = 0;
        i < 10 && find.text('SnapPin-toolbar-qr-test').evaluate().isEmpty;
        i++
      ) {
        await session.settle(tester);
      }
      expect(find.text('SnapPin-toolbar-qr-test'), findsOneWidget);
      expect(tester.takeException(), isNull);
    },
  );
}
