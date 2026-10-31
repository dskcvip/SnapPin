import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/annotation_effects.dart';

void main() {
  final base = Uint8List.fromList([
    for (var y = 0; y < 10; y++)
      for (var x = 0; x < 12; x++) ...[
        (x * 17 + y * 3) % 256,
        (x * 7 + y * 19) % 256,
        (x * 31 + y * 11) % 256,
        128 + (x * 9 + y * 5) % 128,
      ],
  ]);
  const regions = [
    (2, 2, 7, 5),
    (-2, 1, 6, 7),
    (0, 0, 5, 8),
    (0, 0, 12, 10),
    (20, 20, 5, 5),
    (3, 2, 1, 7),
  ];
  for (var n = 0; n < regions.length; n++) {
    final (x, y, w, h) = regions[n];
    test('smart erase matches Tauri RGBA reference, region $n', () {
      final painted = Uint8List.fromList(base);
      for (var py = 0; py < 10; py++) {
        for (var px = 0; px < 12; px++) {
          if (px >= x && px < x + w && py >= y && py < y + h) {
            painted.setRange((py * 12 + px) * 4, (py * 12 + px) * 4 + 4, [
              255,
              0,
              0,
              255,
            ]);
          }
        }
      }
      final actual = AnnotationEffects.apply(
        image: painted,
        base: base,
        width: 12,
        height: 10,
        effect: AnnotationEffect.smartErase,
        x: x,
        y: y,
        regionWidth: w,
        regionHeight: h,
        parameter: 0,
      );
      expect(
        actual,
        File('test/fixtures/tauri-smart-erase-$n.rgba').readAsBytesSync(),
      );
    });
    test('spotlight matches Tauri feather and rounded bounds, region $n', () {
      final actual = AnnotationEffects.apply(
        image: base,
        base: base,
        width: 12,
        height: 10,
        effect: AnnotationEffect.spotlight,
        x: x,
        y: y,
        regionWidth: w,
        regionHeight: h,
        parameter: 150,
        radius: 3,
      );
      final expected = File('test/fixtures/tauri-spotlight-$n.rgba')
          .readAsBytesSync();
      for (var i = 0; i < actual.length; i++) {
        // Rust calculates its SDF in f32; Dart uses f64.
        expect(
          actual[i],
          closeTo(expected[i], i % 4 == 3 ? 0 : 1),
          reason: 'RGBA byte $i',
        );
      }
    });
  }
}
