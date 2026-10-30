import 'package:flutter/material.dart';
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
  });
}
