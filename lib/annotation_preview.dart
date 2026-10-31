import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';

import 'annotation.dart';
import 'windows/mosaic_preview.dart';

/// Interactive effects use the frozen original image, as in Tauri's canvas
/// preview. Export continues to composite the original-resolution pixels.
class AnnotationPreview {
  AnnotationPreview(this.source, this.pixels, this.logicalSize);

  final ui.Image source;
  final ByteData pixels;
  final Size logicalSize;
  final Map<Annotation, ui.Picture> _pictures = {};

  void dispose() {
    for (final picture in _pictures.values) {
      picture.dispose();
    }
    _pictures.clear();
  }

  void paint(Canvas canvas, List<Annotation> marks, Annotation? draft) {
    final retained = marks.toSet();
    for (final mark in _pictures.keys.toList()) {
      if (!retained.contains(mark)) _pictures.remove(mark)!.dispose();
    }
    _original(canvas);
    for (final mark in marks) {
      _paintMark(canvas, mark, cache: true);
    }
    if (draft != null) _paintMark(canvas, draft, cache: false);
  }

  void _original(Canvas canvas) => canvas.drawImageRect(
    source,
    Rect.fromLTWH(0, 0, source.width.toDouble(), source.height.toDouble()),
    Offset.zero & logicalSize,
    Paint()..filterQuality = FilterQuality.none,
  );

  void _paintMark(Canvas canvas, Annotation mark, {required bool cache}) {
    if (mark.points.isEmpty) return;
    if (!mark.isEffect) {
      mark.paint(canvas);
      return;
    }
    if (mark.tool == AnnotationTool.eraser) {
      final path = Path();
      final radius = math.max(1.0, mark.parameter);
      for (var i = 0; i < mark.points.length; i++) {
        final point = mark.points[i];
        path.addOval(Rect.fromCircle(center: point, radius: radius));
        if (i == 0) continue;
        final start = mark.points[i - 1];
        final delta = point - start;
        if (delta.distance == 0) continue;
        final normal = Offset(-delta.dy, delta.dx) / delta.distance * radius;
        path.addPolygon([
          start - normal,
          point - normal,
          point + normal,
          start + normal,
        ], true);
      }
      canvas.save();
      canvas.clipPath(path);
      _original(canvas);
      canvas.restore();
      return;
    }
    if (cache && _pictures.containsKey(mark)) {
      canvas.drawPicture(_pictures[mark]!);
      return;
    }
    final recorder = ui.PictureRecorder();
    final layer = Canvas(recorder);
    final rect = Rect.fromPoints(
      mark.points.first,
      mark.points.last,
    ).intersect(Offset.zero & logicalSize);
    if (!rect.isEmpty) {
      switch (mark.tool) {
        case AnnotationTool.mosaic:
          MosaicPreview(
            source.width,
            source.height,
            pixels.buffer.asUint8List(
              pixels.offsetInBytes,
              pixels.lengthInBytes,
            ),
          ).paint(layer, rect, logicalSize, mark.parameter);
        case AnnotationTool.spotlight:
          // Nested rounded contours give the same outward 1.5px feather as
          // Tauri's preview, without blurring or darkening the hole itself.
          final radius = math.min(12.0, rect.shortestSide / 2);
          final feather = 1.5 * logicalSize.width / source.width;
          for (var step = 0; step < 6; step++) {
            final inset = feather * step / 6;
            final outside = Path()
              ..fillType = PathFillType.evenOdd
              ..addRect(Offset.zero & logicalSize)
              ..addRRect(
                RRect.fromRectAndRadius(
                  rect.inflate(inset),
                  Radius.circular(radius + inset),
                ),
              );
            // Compose six passes to reach the requested total opacity.
            final alpha =
                1 - math.pow(1 - mark.parameter.clamp(0, 255) / 255, 1 / 6);
            layer.drawPath(
              outside,
              Paint()..color = Color.fromRGBO(0, 0, 0, alpha.toDouble()),
            );
          }
        case AnnotationTool.smartErase:
          _smartErase(layer, rect);
        default:
          break;
      }
    }
    final picture = recorder.endRecording();
    canvas.drawPicture(picture);
    if (cache) {
      _pictures[mark] = picture;
    } else {
      picture.dispose();
    }
  }

  void _smartErase(Canvas canvas, Rect rect) {
    // Tauri previews with horizontal boundary gradients while its final
    // original-resolution export uses synchronous wavefront diffusion.
    final sx = source.width / logicalSize.width;
    final sy = source.height / logicalSize.height;
    final left = (rect.left * sx).round().clamp(0, source.width - 1);
    final right = (rect.right * sx).round().clamp(0, source.width);
    final top = (rect.top * sy).round().clamp(0, source.height - 1);
    final bottom = (rect.bottom * sy).round().clamp(0, source.height);
    Color at(int x, int y) {
      final offset = (y * source.width + x) * 4;
      return Color.fromARGB(
        pixels.getUint8(offset + 3),
        pixels.getUint8(offset),
        pixels.getUint8(offset + 1),
        pixels.getUint8(offset + 2),
      );
    }

    canvas.save();
    canvas.clipRect(rect);
    for (var y = top; y < bottom; y++) {
      final row = Rect.fromLTWH(rect.left, y / sy, rect.width, 1 / sy + .05);
      canvas.drawRect(
        row,
        Paint()
          ..shader = ui.Gradient.linear(
            Offset(rect.left, 0),
            Offset(rect.right, 0),
            [
              at(math.max(0, left - 1), y),
              at(math.min(source.width - 1, right), y),
            ],
          ),
      );
    }
    canvas.restore();
  }
}
