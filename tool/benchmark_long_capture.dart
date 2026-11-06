import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:image/image.dart' as img;
import 'package:snappin_flutter/long_stitch.dart';

// Compare codec overhead at the dimensions from the reported Intel Mac log.
// Synthetic pixels only; this does not measure CoreGraphics or claim Mac FPS.
void main() {
  const width = 3344, height = 1834;
  Uint8List frame(int top) {
    final pixels = Uint8List(width * height * 4);
    for (var y = 0; y < height; y++) {
      final row = top + y;
      final seed = (row * 1103515245 + 12345) & 0x7fffffff;
      for (var x = 0; x < width; x++) {
        final index = (y * width + x) * 4;
        pixels[index] = ((seed >> 16) + x * 5) % 256;
        pixels[index + 1] = ((seed >> 9) + x * 3) % 256;
        pixels[index + 2] = ((seed >> 2) + x * 7) % 256;
        pixels[index + 3] = 255;
      }
    }
    return pixels;
  }

  final positions = [0, 800, 1600, 800, 0, 1200, 2400];
  final raw = positions.map(frame).toList();
  final png = raw
      .map(
        (pixels) => Uint8List.fromList(
          img.encodePng(
            img.Image.fromBytes(
              width: width,
              height: height,
              bytes: pixels.buffer,
              numChannels: 4,
            ),
          ),
        ),
      )
      .toList();
  final results = <String, Object>{'width': width, 'frameHeight': height};
  for (final useRaw in [false, true]) {
    final session = useRaw
        ? LongStitchSession.startRgba(raw.first, width, height)
        : LongStitchSession.start(png.first);
    final timings = <int>[];
    try {
      for (var index = 1; index < raw.length; index++) {
        final watch = Stopwatch()..start();
        final step = useRaw
            ? session.pushRgba(raw[index], width, height)
            : session.push(png[index]);
        timings.add(watch.elapsedMicroseconds);
        if (step.kind == LongStepKind.noMatch) throw StateError('Lost overlap');
      }
      if (session.width != width || session.height != height + 2400) {
        throw StateError('Image dimensions changed');
      }
      results[useRaw ? 'raw_us' : 'png_us'] = timings;
    } finally {
      session.dispose();
    }
  }
  stdout.writeln(jsonEncode(results));
}
