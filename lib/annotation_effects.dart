import 'dart:math' as math;
import 'dart:typed_data';

enum AnnotationEffect { mosaic, spotlight, eraser, smartErase }

class AnnotationEffects {
  AnnotationEffects._();

  static Uint8List apply({
    required Uint8List image,
    required Uint8List base,
    required int width,
    required int height,
    required AnnotationEffect effect,
    required int x,
    required int y,
    required int regionWidth,
    required int regionHeight,
    required double parameter,
    int radius = 0,
    List<double> points = const [],
  }) {
    if (image.length != width * height * 4 || base.length != image.length) {
      throw ArgumentError('Annotation image size does not match RGBA buffer');
    }
    final output = Uint8List.fromList(image);
    final left = x.clamp(0, width);
    final top = y.clamp(0, height);
    final right = (x + regionWidth).clamp(0, width);
    final bottom = (y + regionHeight).clamp(0, height);
    switch (effect) {
      case AnnotationEffect.mosaic:
        final block = math.max(2, parameter.round());
        for (var by = top; by < bottom; by += block) {
          for (var bx = left; bx < right; bx += block) {
            final sums = [0, 0, 0, 0];
            var count = 0;
            final yEnd = math.min(by + block, bottom);
            final xEnd = math.min(bx + block, right);
            for (var py = by; py < yEnd; py++) {
              for (var px = bx; px < xEnd; px++) {
                final offset = (py * width + px) * 4;
                for (var c = 0; c < 4; c++) sums[c] += image[offset + c];
                count++;
              }
            }
            for (var py = by; py < yEnd; py++) {
              for (var px = bx; px < xEnd; px++) {
                final offset = (py * width + px) * 4;
                for (var c = 0; c < 4; c++) {
                  output[offset + c] = sums[c] ~/ count;
                }
              }
            }
          }
        }
      case AnnotationEffect.spotlight:
        final dim = parameter.round().clamp(0, 255);
        final r = math.max(0, radius);
        for (var py = 0; py < height; py++) {
          for (var px = 0; px < width; px++) {
            final dx = px < left ? left - px : px >= right ? px - right + 1 : 0;
            final dy = py < top ? top - py : py >= bottom ? py - bottom + 1 : 0;
            if (dx * dx + dy * dy <= r * r) continue;
            final offset = (py * width + px) * 4;
            for (var c = 0; c < 3; c++) {
              output[offset + c] = image[offset + c] * (255 - dim) ~/ 255;
            }
          }
        }
      case AnnotationEffect.eraser:
        final brush = math.max(1, parameter.round());
        for (var i = 0; i + 1 < points.length; i += 2) {
          final cx = points[i].round();
          final cy = points[i + 1].round();
          for (var py = math.max(0, cy - brush);
              py < math.min(height, cy + brush + 1); py++) {
            for (var px = math.max(0, cx - brush);
                px < math.min(width, cx + brush + 1); px++) {
              final dx = px - cx;
              final dy = py - cy;
              if (dx * dx + dy * dy > brush * brush) continue;
              final offset = (py * width + px) * 4;
              output.setRange(offset, offset + 4, base, offset);
            }
          }
        }
      case AnnotationEffect.smartErase:
        if (left >= right || top >= bottom ||
            (left == 0 && top == 0 && right == width && bottom == height)) {
          return output;
        }
        // Fill from the closest undamaged edge. This is local and deterministic.
        for (var py = top; py < bottom; py++) {
          for (var px = left; px < right; px++) {
            final distances = [px - left + 1, right - px, py - top + 1, bottom - py];
            final nearest = distances.reduce(math.min);
            final sourceX = nearest == distances[0] ? left - 1 :
                nearest == distances[1] ? right : px;
            final sourceY = nearest == distances[2] ? top - 1 :
                nearest == distances[3] ? bottom : py;
            final sx = sourceX.clamp(0, width - 1);
            final sy = sourceY.clamp(0, height - 1);
            final target = (py * width + px) * 4;
            final source = (sy * width + sx) * 4;
            output.setRange(target, target + 4, base, source);
          }
        }
    }
    return output;
  }
}
