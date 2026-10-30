import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/app_config.dart';
import 'package:snappin_flutter/windows/models_settings.dart';

void main() {
  testWidgets('model preset fills required fields and marks the active model', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1000, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    var config = const AppConfig();
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: StatefulBuilder(
            builder: (context, update) => ModelsSettings(
              config: config,
              onChanged: (next) => update(() => config = next),
            ),
          ),
        ),
      ),
    );

    await tester.tap(find.text('添加模型'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<FilledButton>(find.widgetWithText(FilledButton, '确定'))
          .onPressed,
      isNull,
    );

    await tester.tap(find.byType(DropdownButtonFormField<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('智谱 GLM · BigModel 开放平台').last);
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<FilledButton>(find.widgetWithText(FilledButton, '确定'))
          .onPressed,
      isNotNull,
    );

    await tester.tap(find.widgetWithText(FilledButton, '确定'));
    await tester.pumpAndSettle();
    expect(config.translationModels.single.name, '智谱 GLM');
    expect(config.translationModels.single.model, 'glm-4-flash');
    expect(find.text('使用中'), findsOneWidget);
    final originalId = config.translationModels.single.id;

    await tester.tap(find.byTooltip('编辑'));
    await tester.pumpAndSettle();
    expect(find.byType(DropdownButtonFormField<String>), findsNothing);
    await tester.enterText(find.byType(TextField).first, '未确认的修改');
    await tester.tap(find.widgetWithText(OutlinedButton, '取消'));
    await tester.pumpAndSettle();
    expect(config.translationModels.single.name, '智谱 GLM');

    await tester.tap(find.byTooltip('编辑'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).first, '已确认的修改');
    await tester.tap(find.widgetWithText(FilledButton, '确定'));
    await tester.pumpAndSettle();
    expect(config.translationModels.single.id, originalId);
    expect(config.translationModels.single.name, '已确认的修改');

    await tester.tap(find.text('添加模型'));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(DropdownButtonFormField<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Kimi · Moonshot 月之暗面').last);
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, '确定'));
    await tester.pumpAndSettle();
    expect(config.translationModels.map((m) => m.name), ['已确认的修改', 'Kimi']);
    expect(config.translationModels.first.id, originalId);
    expect(find.text('使用中'), findsOneWidget);

    // Escape dismisses only the draft editor; it does not commit its inputs.
    await tester.tap(find.byTooltip('编辑').last);
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).first, '放弃的草稿');
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(config.translationModels.last.name, 'Kimi');

    await tester.tap(find.byTooltip('删除').first);
    await tester.pumpAndSettle();
    expect(config.translationModels.single.name, 'Kimi');
    expect(find.text('使用中'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });
}
