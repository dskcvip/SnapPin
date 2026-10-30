import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/shot_icons.dart';

void main() {
  testWidgets('capture toolbar icons load from bundled assets', (tester) async {
    const names = [
      'rectangle', 'ellipse', 'arrow', 'pen', 'translate', 'ocr', 'qr',
      'undo', 'redo', 'save', 'pin', 'close', 'check',
    ];
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: Row(children: [
          for (final name in names) ShotIcon(name),
        ]),
      ),
    ));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
  });
}
