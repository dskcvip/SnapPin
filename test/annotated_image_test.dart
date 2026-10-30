import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/annotation.dart';
import 'package:snappin_flutter/annotated_image.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test(
    'long image edits retain full size, bottom pixels and tile seams',
    () async {
      final source = img.Image(width: 40, height: 17020, numChannels: 4);
      img.fill(source, color: img.ColorRgba8(255, 255, 255, 255));
      img.fillRect(
        source,
        x1: 0,
        y1: 16950,
        x2: 39,
        y2: 17019,
        color: img.ColorRgba8(0, 0, 255, 255),
      );
      final png = await renderAnnotatedImage(
        Uint8List.fromList(img.encodePng(source)),
        const Rect.fromLTWH(0, 10, 40, 17000),
        const [
          Annotation(
            tool: AnnotationTool.rectangle,
            points: [Offset(5, 16380), Offset(30, 16420)],
            color: Colors.red,
            strokeWidth: 4,
          ),
        ],
      );
      final output = img.decodePng(png)!;
      expect(output.width, 40);
      expect(output.height, 17000);
      for (final y in [16373, 16384, 16400]) {
        final pixel = output.getPixel(5, y);
        expect(pixel.r.toInt(), greaterThan(200));
        expect(pixel.g.toInt(), lessThan(100));
        expect(pixel.a.toInt(), 255);
      }
      final bottom = output.getPixel(20, 16999);
      expect(bottom.b.toInt(), 255);
      expect(bottom.r.toInt(), 0);
      expect(bottom.a.toInt(), 255);
    },
  );
}
