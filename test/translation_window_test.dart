import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/windows/translation_window.dart';

void main() {
  testWidgets(
    'translation window keeps both panes and select-all controls usable',
    (tester) async {
      tester.view.physicalSize = const Size(820, 550);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);

      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(body: TranslationWindow(source: '第一行\n第二行')),
        ),
      );
      await tester.pump();

      expect(find.text('全选原文'), findsOneWidget);
      expect(find.text('全选译文'), findsOneWidget);
      expect(find.text('原文'), findsOneWidget);
      expect(find.text('译文'), findsOneWidget);

      await tester.tap(find.text('全选原文'));
      await tester.pump();
      final source = tester.widget<TextField>(find.byType(TextField).first);
      expect(source.controller!.selection.baseOffset, 0);
      expect(
        source.controller!.selection.extentOffset,
        source.controller!.text.length,
      );
      expect(tester.takeException(), isNull);
    },
  );
}
