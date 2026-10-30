import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/annotation_effects.dart';

void main() {
  test('native annotation effects preserve the original image for erasing', () {
    final base = Uint8List.fromList([
      for (var i = 0; i < 64; i++) ...[100, 120, 140, 255],
    ]);
    final marked = Uint8List.fromList(base);
    marked[(3 * 8 + 3) * 4] = 240;
    final erased = AnnotationEffects.apply(
      image: marked,
      base: base,
      width: 8,
      height: 8,
      effect: AnnotationEffect.eraser,
      x: 0,
      y: 0,
      regionWidth: 0,
      regionHeight: 0,
      parameter: 2,
      points: const [3, 3],
    );
    expect(erased, base);

    final spotlight = AnnotationEffects.apply(
      image: base,
      base: base,
      width: 8,
      height: 8,
      effect: AnnotationEffect.spotlight,
      x: 2,
      y: 2,
      regionWidth: 4,
      regionHeight: 4,
      parameter: 150,
      radius: 0,
    );
    expect(spotlight[0], lessThan(base[0]));
    expect(spotlight[(3 * 8 + 3) * 4], base[(3 * 8 + 3) * 4]);
  });
}
