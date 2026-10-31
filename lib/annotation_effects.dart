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
        if (regionWidth <= 0 || regionHeight <= 0 || dim == 0) return output;
        final r = math
            .max(
              0,
              math.min(radius, math.min(regionWidth ~/ 2, regionHeight ~/ 2)),
            )
            .toDouble();
        final cx = x + regionWidth / 2;
        final cy = y + regionHeight / 2;
        final hx = regionWidth / 2 - r;
        final hy = regionHeight / 2 - r;
        for (var py = 0; py < height; py++) {
          for (var px = 0; px < width; px++) {
            final qx = (px + .5 - cx).abs() - hx;
            final qy = (py + .5 - cy).abs() - hy;
            final dx = math.max(0.0, qx);
            final dy = math.max(0.0, qy);
            final distance =
                math.sqrt(dx * dx + dy * dy) +
                math.min(math.max(qx, qy), 0) -
                r;
            final strength = distance.clamp(0, 1.5) / 1.5;
            if (strength == 0) continue;
            final offset = (py * width + px) * 4;
            for (var c = 0; c < 3; c++) {
              output[offset +
                  c] = (image[offset + c] * (1 - dim / 255 * strength))
                  .round()
                  .clamp(0, 255);
            }
          }
        }
      case AnnotationEffect.eraser:
        final brush = math.max(1, parameter.round());
        void restoreAt(double x, double y) {
          final cx = x.round();
          final cy = y.round();
          for (
            var py = math.max(0, cy - brush);
            py < math.min(height, cy + brush + 1);
            py++
          ) {
            for (
              var px = math.max(0, cx - brush);
              px < math.min(width, cx + brush + 1);
              px++
            ) {
              final dx = px - cx;
              final dy = py - cy;
              if (dx * dx + dy * dy > brush * brush) continue;
              final offset = (py * width + px) * 4;
              output.setRange(offset, offset + 4, base, offset);
            }
          }
        }
        if (points.length >= 2) restoreAt(points[0], points[1]);
        for (var i = 2; i + 1 < points.length; i += 2) {
          final startX = points[i - 2];
          final startY = points[i - 1];
          final dx = points[i] - startX;
          final dy = points[i + 1] - startY;
          // Pointer events can be far apart during a fast drag. Restore the
          // whole stroke between samples so exported annotations have no gaps.
          final steps = math.max(
            1,
            (math.sqrt(dx * dx + dy * dy) / math.max(1, brush / 2)).ceil(),
          );
          for (var step = 1; step <= steps; step++) {
            restoreAt(startX + dx * step / steps, startY + dy * step / steps);
          }
        }
      case AnnotationEffect.smartErase:
        if (left >= right ||
            top >= bottom ||
            (left == 0 && top == 0 && right == width && bottom == height)) {
          return output;
        }
        _diffuse(output, base, width, height, left, top, right, bottom);
    }
    return output;
  }

  // Tauri smart_erase_region: seed from the original outside boundary, then
  // fill one synchronous frontier per round. Newly queued pixels cannot feed
  // their neighbours until the following round (avoids directional streaks).
  static void _diffuse(
    Uint8List output,
    Uint8List base,
    int width,
    int height,
    int left,
    int top,
    int right,
    int bottom,
  ) {
    final rw = right - left;
    final rh = bottom - top;
    final state = Uint8List(rw * rh); // 0 unfilled, 1 queued, 2 filled
    const directions = [(-1, 0), (1, 0), (0, -1), (0, 1)];
    bool fill(int lx, int ly, {required bool seed}) {
      var sr = 0, sg = 0, sb = 0, sa = 0, count = 0;
      for (final (dx, dy) in directions) {
        final nx = lx + dx;
        final ny = ly + dy;
        final outside = nx < 0 || ny < 0 || nx >= rw || ny >= rh;
        final ax = left + nx;
        final ay = top + ny;
        if (ax < 0 || ay < 0 || ax >= width || ay >= height) continue;
        if (!outside && (seed || state[ny * rw + nx] != 2)) continue;
        final data = outside ? base : output;
        final offset = (ay * width + ax) * 4;
        sr += data[offset];
        sg += data[offset + 1];
        sb += data[offset + 2];
        sa += data[offset + 3];
        count++;
      }
      if (count == 0) return false;
      final target = ((top + ly) * width + left + lx) * 4;
      output[target] = sr ~/ count;
      output[target + 1] = sg ~/ count;
      output[target + 2] = sb ~/ count;
      output[target + 3] = sa ~/ count;
      return true;
    }

    var frontier = <int>[];
    for (var ly = 0; ly < rh; ly++) {
      for (var lx = 0; lx < rw; lx++) {
        if (lx != 0 && ly != 0 && lx != rw - 1 && ly != rh - 1) continue;
        if (fill(lx, ly, seed: true)) {
          final index = ly * rw + lx;
          state[index] = 2;
          frontier.add(index);
        }
      }
    }
    for (var round = 0; round < math.min(rw, rh) ~/ 2 + 2; round++) {
      if (frontier.isEmpty) break;
      final next = <int>[];
      for (final index in frontier) {
        final lx = index % rw;
        final ly = index ~/ rw;
        for (final (dx, dy) in directions) {
          final nx = lx + dx;
          final ny = ly + dy;
          if (nx < 0 || ny < 0 || nx >= rw || ny >= rh) continue;
          final q = ny * rw + nx;
          if (state[q] != 0) continue;
          state[q] = 1;
          if (fill(nx, ny, seed: false)) {
            next.add(q);
          } else {
            state[q] = 0;
          }
        }
      }
      for (final index in next) {
        state[index] = 2;
      }
      frontier = next;
    }
  }
}
