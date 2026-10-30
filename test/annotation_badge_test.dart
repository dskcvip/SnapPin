import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/annotation.dart';

void main() {
  test('badge is centered on the pointer position', () async {
    const point = Offset(50, 50);
    final recorder = ui.PictureRecorder();
    const Annotation(
      tool: AnnotationTool.badge,
      points: [point],
      color: Colors.red,
      strokeWidth: 20,
      number: 1,
    ).paint(Canvas(recorder));
    final image = await recorder.endRecording().toImage(100, 100);
    final bytes = (await image.toByteData(format: ui.ImageByteFormat.rawRgba))!;
    int alphaAt(int x, int y) => bytes.getUint8((y * 100 + x) * 4 + 3);

    expect(alphaAt(50, 50), 255);
    expect(alphaAt(33, 50), 255);
    expect(alphaAt(67, 50), 255);
    expect(alphaAt(30, 30), 0);
    image.dispose();
  });
}
