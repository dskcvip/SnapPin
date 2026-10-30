import 'dart:async';
import 'dart:math' as math;
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';

import 'annotation.dart';
import 'annotation_effects.dart';

Future<Uint8List> cropSelection(
  ui.Image source,
  Rect selection,
  Size logicalSize, [
  List<Annotation> annotations = const [],
]) async {
  if (selection.width < 3 || selection.height < 3) {
    throw ArgumentError.value(selection, 'selection', 'Selection is too small');
  }
  final sx = source.width / logicalSize.width;
  final sy = source.height / logicalSize.height;
  final src =
      Rect.fromLTRB(
        selection.left * sx,
        selection.top * sy,
        selection.right * sx,
        selection.bottom * sy,
      ).intersect(
        Rect.fromLTWH(0, 0, source.width.toDouble(), source.height.toDouble()),
      );
  final width = math.max(1, src.width.round());
  final height = math.max(1, src.height.round());
  final baseRecorder = ui.PictureRecorder();
  final baseCanvas = Canvas(baseRecorder);
  baseCanvas.drawImageRect(
    source,
    src,
    Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble()),
    Paint()..filterQuality = FilterQuality.none,
  );
  final base = await baseRecorder.endRecording().toImage(width, height);
  var current = base;
  try {
    for (final annotation in annotations) {
      ui.Image next;
      if (annotation.isEffect) {
        final currentData = await current.toByteData(
          format: ui.ImageByteFormat.rawRgba,
        );
        final baseData = await base.toByteData(
          format: ui.ImageByteFormat.rawRgba,
        );
        if (currentData == null || baseData == null)
          throw StateError('RGBA conversion failed');
        final a = annotation.points.first;
        final b = annotation.points.last;
        final left = ((math.min(a.dx, b.dx) - selection.left) * sx).round();
        final top = ((math.min(a.dy, b.dy) - selection.top) * sy).round();
        final regionWidth = (a.dx - b.dx).abs() * sx;
        final regionHeight = (a.dy - b.dy).abs() * sy;
        final effect = switch (annotation.tool) {
          AnnotationTool.mosaic => AnnotationEffect.mosaic,
          AnnotationTool.spotlight => AnnotationEffect.spotlight,
          AnnotationTool.eraser => AnnotationEffect.eraser,
          AnnotationTool.smartErase => AnnotationEffect.smartErase,
          _ => throw StateError('Unsupported annotation effect'),
        };
        final output = AnnotationEffects.apply(
          image: currentData.buffer.asUint8List(
            currentData.offsetInBytes,
            currentData.lengthInBytes,
          ),
          base: baseData.buffer.asUint8List(
            baseData.offsetInBytes,
            baseData.lengthInBytes,
          ),
          width: width,
          height: height,
          effect: effect,
          x: left,
          y: top,
          regionWidth: regionWidth.round(),
          regionHeight: regionHeight.round(),
          parameter:
              annotation.parameter *
              (effect == AnnotationEffect.spotlight ? 1 : sx),
          radius: (12 * sx).round(),
          points: [
            for (final point in annotation.points) ...[
              (point.dx - selection.left) * sx,
              (point.dy - selection.top) * sy,
            ],
          ],
        );
        final ready = Completer<ui.Image>();
        ui.decodeImageFromPixels(
          output,
          width,
          height,
          ui.PixelFormat.rgba8888,
          ready.complete,
        );
        next = await ready.future;
      } else {
        final recorder = ui.PictureRecorder();
        final canvas = Canvas(recorder);
        canvas.drawImage(current, Offset.zero, Paint());
        canvas.save();
        canvas.clipRect(
          Rect.fromLTWH(0, 0, width.toDouble(), height.toDouble()),
        );
        canvas.scale(sx, sy);
        canvas.translate(-selection.left, -selection.top);
        annotation.paint(canvas);
        canvas.restore();
        next = await recorder.endRecording().toImage(width, height);
      }
      if (!identical(current, base)) current.dispose();
      current = next;
    }
    final data = await current.toByteData(format: ui.ImageByteFormat.png);
    if (data == null) throw StateError('Image encoding failed');
    return data.buffer.asUint8List(data.offsetInBytes, data.lengthInBytes);
  } finally {
    if (!identical(current, base)) current.dispose();
    base.dispose();
  }
}
