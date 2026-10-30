import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/windows/capture_exit_dialog.dart';

void main() {
  test('do not ask preference survives recreation', () async {
    final directory = await Directory.systemTemp.createTemp(
      'snappin-exit-test-',
    );
    addTearDown(() => directory.delete(recursive: true));
    final file = File('${directory.path}/preference.json');
    expect(await CaptureExitPreference(file).shouldAsk(), isTrue);
    await CaptureExitPreference(file).stopAsking();
    expect(await CaptureExitPreference(file).shouldAsk(), isFalse);
  });

  testWidgets(
    'exit confirmation preserves screenshot on No and remembers choice only on Yes',
    (tester) async {
      CaptureExitDecision? result;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: Builder(
              builder: (context) => TextButton(
                onPressed: () async =>
                    result = await showDialog<CaptureExitDecision>(
                      context: context,
                      builder: (_) => const CaptureExitDialog(),
                    ),
                child: const Text('Open'),
              ),
            ),
          ),
        ),
      );
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      expect(find.text('确认退出截图？'), findsOneWidget);
      await tester.tap(find.text('不再询问'));
      await tester.tap(find.text('否'));
      await tester.pumpAndSettle();
      expect(result?.exit, isFalse);
      expect(result?.dontAskAgain, isFalse);
      await tester.tap(find.text('Open'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('不再询问'));
      await tester.tap(find.text('是'));
      await tester.pumpAndSettle();
      expect(result?.exit, isTrue);
      expect(result?.dontAskAgain, isTrue);
      expect(tester.takeException(), isNull);
    },
  );
}
