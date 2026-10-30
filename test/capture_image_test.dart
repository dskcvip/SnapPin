import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/annotation.dart';
import 'package:snappin_flutter/capture_image.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  test('retina selection exports the matching physical pixels', () async {
    final recorder = ui.PictureRecorder();
    final canvas = Canvas(recorder);
    canvas.drawRect(
      const Rect.fromLTWH(0, 0, 10, 20),
      Paint()..color = const Color(0xffff0000),
    );
    canvas.drawRect(
      const Rect.fromLTWH(10, 0, 10, 20),
      Paint()..color = const Color(0xff0000ff),
    );
    final source = await recorder.endRecording().toImage(20, 20);

    final png = await cropSelection(
      source,
      const Rect.fromLTWH(5, 0, 5, 10),
      const Size(10, 10),
    );
    final codec = await ui.instantiateImageCodec(png);
    final cropped = (await codec.getNextFrame()).image;
    expect(cropped.width, 10);
    expect(cropped.height, 20);
    final pixels = await cropped.toByteData(format: ui.ImageByteFormat.rawRgba);
    expect(pixels, isNotNull);
    for (var offset = 0; offset < pixels!.lengthInBytes; offset += 4) {
      expect(pixels.getUint8(offset), 0);
      expect(pixels.getUint8(offset + 1), 0);
      expect(pixels.getUint8(offset + 2), 255);
      expect(pixels.getUint8(offset + 3), 255);
    }
    cropped.dispose();
    source.dispose();
  });

  test('annotations are composited at physical resolution', () async {
    final recorder = ui.PictureRecorder();
    Canvas(recorder).drawColor(Colors.white, BlendMode.src);
    final source = await recorder.endRecording().toImage(20, 20);
    final png = await cropSelection(
      source,
      const Rect.fromLTWH(0, 0, 10, 10),
      const Size(10, 10),
      [
        const Annotation(
          tool: AnnotationTool.rectangle,
          points: [Offset(3, 3), Offset(7, 7)],
          color: Color(0xffff0000),
          strokeWidth: 1,
        ),
      ],
    );
    final codec = await ui.instantiateImageCodec(png);
    final image = (await codec.getNextFrame()).image;
    final pixels = (await image.toByteData(
      format: ui.ImageByteFormat.rawRgba,
    ))!;
    int channel(int x, int y, int index) =>
        pixels.getUint8((y * image.width + x) * 4 + index);
    expect(channel(6, 6, 0), greaterThan(200));
    expect(channel(6, 6, 1), lessThan(100));
    expect(channel(10, 10, 1), 255);
    image.dispose();
    source.dispose();
  });
}
