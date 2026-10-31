import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/ocr_document.dart';

void main() {
  final data = {
    'text': 'legacy column order',
    'width': 850,
    'height': 2000,
    'lines': [
      {'text': '右列', 'x': 620, 'y': 100, 'width': 80, 'height': 20},
      {'text': '第二行', 'x': 30, 'y': 200, 'width': 100, 'height': 20},
      {'text': '第一行', 'x': 30, 'y': 100, 'width': 100, 'height': 20},
    ],
  };
  test('layout IPC keeps geometry and restores reading order', () {
    final document = OcrDocument.decode(jsonEncode(data));
    expect(document.text, '第一行\n右列\n第二行');
    expect(document.lines[1].bounds.left, 620);
    expect(
      OcrDocument.decode(jsonEncode(document.toJson())).toJson(),
      document.toJson(),
    );
  });
  testWidgets('layout preserves separate columns and vertical spacing', (
    tester,
  ) async {
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: OcrLayoutView(document: OcrDocument.fromJson(data)),
        ),
      ),
    );
    final left = tester.getTopLeft(find.text('第一行'));
    final right = tester.getTopLeft(find.text('右列'));
    final second = tester.getTopLeft(find.text('第二行'));
    expect(right.dx - left.dx, 590);
    expect(right.dy, left.dy);
    expect(second.dy - left.dy, 100);
    expect(tester.takeException(), isNull);
  });
}
