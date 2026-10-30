import 'dart:io';
import 'dart:ui' as ui;

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/app_config.dart';
import 'package:snappin_flutter/windows/settings_window.dart';

void main() {
  testWidgets('all settings pages fit the Tauri reference window', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(782, 640);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(
      const MethodChannel('mixin.one/desktop_multi_window/channels'),
      (call) async => null,
    );
    messenger.setMockMethodCallHandler(
      const MethodChannel('mixin.one/desktop_multi_window'),
      (call) async => call.method == 'getWindowDefinition'
          ? {'windowId': 'settings-test', 'windowArgument': ''}
          : null,
    );
    final managerCalls = <String>[];
    messenger.setMockMethodCallHandler(const MethodChannel('window_manager'), (
      call,
    ) async {
      managerCalls.add(call.method);
      return null;
    });
    final review = Platform.environment['SNAPPIN_VISUAL_REVIEW'];
    if (review != null) {
      debugDisableShadows = false;
      addTearDown(() => debugDisableShadows = true);
      await tester.runAsync(() async {
        for (final font in [
          ('Segoe UI', r'C:\Windows\Fonts\segoeui.ttf'),
          ('Microsoft YaHei', r'C:\Windows\Fonts\msyh.ttc'),
          ('Consolas', r'C:\Windows\Fonts\consola.ttf'),
        ]) {
          final loader = FontLoader(font.$1)
            ..addFont(
              File(font.$2)
                  .readAsBytes()
                  .then((bytes) => bytes.buffer.asByteData()),
            );
          await loader.load();
        }
      });
    }
    final boundary = GlobalKey();
    final appBoundary = GlobalKey();
    await tester.pumpWidget(
      MaterialApp(
        builder: (context, child) =>
            RepaintBoundary(key: appBoundary, child: child!),
        theme: ThemeData(
          useMaterial3: false,
          fontFamily: 'Segoe UI',
          fontFamilyFallback: const ['Microsoft YaHei'],
          colorScheme: const ColorScheme.light(primary: Color(0xff3ca6d8)),
        ),
        home: Scaffold(
          body: RepaintBoundary(
            key: boundary,
            child: SettingsWindow(
              controller: WindowController.fromWindowId('settings-test'),
              initialConfig: const AppConfig(
                ballSize: 50,
                ballOpacity: .6,
                themeColor: '#3CA6D8',
                translationModels: [
                  TranslationModel(
                    id: 'test',
                    name: 'deepseek-flash',
                    note: '',
                    baseUrl: 'https://api.deepseek.com',
                    apiKey: 'test-key',
                    model: 'deepseek-chat',
                    keyUrl: 'https://platform.deepseek.com/api_keys',
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    for (final page in ['通用', '快捷键', '模型', '数据管理', '关于']) {
      if (page != '通用') {
        await tester.tap(find.text(page).first);
        await tester.pumpAndSettle();
      }
      expect(tester.takeException(), isNull, reason: '$page layout');
      if (review != null) {
        await tester.runAsync(() async {
          final image =
              await (boundary.currentContext!.findRenderObject()!
                      as RenderRepaintBoundary)
                  .toImage();
          final data = (await image.toByteData(
            format: ui.ImageByteFormat.png,
          ))!;
          final file = File('$review/$page.png');
          await file.parent.create(recursive: true);
          await file.writeAsBytes(data.buffer.asUint8List());
          image.dispose();
        });
      }
    }
    await tester.tap(find.text('模型').first);
    await tester.pumpAndSettle();
    final card = find.byKey(const ValueKey('model-list-card'));
    expect(tester.getTopLeft(card).dx, closeTo(233, 1));
    expect(tester.getTopLeft(card).dy, closeTo(99, 1));
    await tester.tap(find.byTooltip('编辑'));
    await tester.pumpAndSettle();
    final editor = find.byKey(const ValueKey('model-editor-surface'));
    expect(tester.getSize(editor).width, 460);
    expect(tester.getSize(editor).height, closeTo(384, 1));
    expect(find.byType(TextField), findsNWidgets(6));
    for (final field in find.byType(TextField).evaluate()) {
      expect(tester.getSize(find.byWidget(field.widget)).height, 32);
    }
    expect(
      tester.widget<TextField>(find.byType(TextField).at(3)).obscureText,
      isTrue,
    );
    if (review != null) {
      await tester.runAsync(() async {
        final image =
            await (appBoundary.currentContext!.findRenderObject()!
                    as RenderRepaintBoundary)
                .toImage();
        final data = (await image.toByteData(format: ui.ImageByteFormat.png))!;
        await File('$review/模型编辑.png').writeAsBytes(data.buffer.asUint8List());
        image.dispose();
      });
    }
    await tester.tap(find.widgetWithText(OutlinedButton, '取消'));
    await tester.pumpAndSettle();
    expect(find.text('deepseek-flash'), findsOneWidget);
    await tester.tap(find.text('通用').first);
    await tester.pumpAndSettle();
    expect(tester.getTopLeft(find.text('通用').last).dy, closeTo(20, 1));
    final language = find.byType(TextFormField);
    expect(tester.getSize(language).height, 32);
    await tester.enterText(language, '英文');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.text('有未保存的修改'), findsOneWidget);
    expect(managerCalls, isNot(contains('destroy')));
    expect(managerCalls, isNot(contains('close')));
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
    expect(find.text('有未保存的修改'), findsNothing);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    await tester.tap(find.text('放弃并关闭'));
    await tester.pumpAndSettle();
    expect(managerCalls, contains('hide'));
    expect(managerCalls, isNot(contains('close')));
    expect(managerCalls, isNot(contains('destroy')));
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    debugDisableShadows = true;
  });
}
