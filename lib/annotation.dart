import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

enum AnnotationTool {
  rectangle,
  ellipse,
  sticker,
  arrow,
  pen,
  highlighter,
  mosaic,
  spotlight,
  text,
  badge,
  eraser,
  watermark,
  smartErase,
}

class Annotation {
  const Annotation({
    required this.tool,
    required this.points,
    required this.color,
    required this.strokeWidth,
    this.text,
    this.number,
    this.parameter = 0,
    this.stickerPicture,
    this.stickerPictureSize,
  });

  final AnnotationTool tool;
  final List<Offset> points;
  final Color color;
  final double strokeWidth;
  final String? text;
  final int? number;
  final double parameter;
  final ui.Picture? stickerPicture;
  final Size? stickerPictureSize;

  bool get isEffect => switch (tool) {
    AnnotationTool.mosaic ||
    AnnotationTool.spotlight ||
    AnnotationTool.eraser ||
    AnnotationTool.smartErase => true,
    _ => false,
  };

  void paint(Canvas canvas) {
    if (points.isEmpty) return;
    final brush = Paint()
      ..color = color
      ..strokeWidth = strokeWidth
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round
      ..style = PaintingStyle.stroke;
    final start = points.first;
    final end = points.last;
    switch (tool) {
      case AnnotationTool.rectangle:
        canvas.drawRect(Rect.fromPoints(start, end), brush);
      case AnnotationTool.ellipse:
        canvas.drawOval(Rect.fromPoints(start, end), brush);
      case AnnotationTool.arrow:
        canvas.drawLine(start, end, brush);
        final angle = math.atan2(end.dy - start.dy, end.dx - start.dx);
        final head = math.max(10.0, strokeWidth * 4);
        for (final side in [-1, 1]) {
          final direction = angle + side * math.pi / 6;
          canvas.drawLine(
            end,
            end - Offset(math.cos(direction), math.sin(direction)) * head,
            brush,
          );
        }
      case AnnotationTool.pen:
      case AnnotationTool.highlighter:
        if (tool == AnnotationTool.highlighter) {
          brush
            ..color = const Color(0x5afacc15)
            ..strokeWidth = 14;
        }
        final path = Path()..moveTo(start.dx, start.dy);
        for (final point in points.skip(1)) {
          path.lineTo(point.dx, point.dy);
        }
        if (points.length == 1) path.lineTo(start.dx + .01, start.dy + .01);
        canvas.drawPath(path, brush);
      case AnnotationTool.text:
        _drawText(canvas, text ?? '', start, strokeWidth, color);
      case AnnotationTool.sticker:
        if (stickerPicture != null && stickerPictureSize != null) {
          canvas.save();
          canvas.translate(start.dx, start.dy);
          canvas.scale(
            strokeWidth / stickerPictureSize!.width,
            strokeWidth / stickerPictureSize!.height,
          );
          canvas.drawPicture(stickerPicture!);
          canvas.restore();
        } else {
          _drawText(
            canvas,
            text ?? '✅',
            start,
            strokeWidth,
            color,
            emoji: true,
          );
        }
      case AnnotationTool.badge:
        final label = '${number ?? 1}';
        final diameter = math.max(
          strokeWidth * 1.9,
          label.length * strokeWidth * .9,
        );
        final center = start + Offset(diameter / 2, diameter / 2);
        canvas.drawCircle(center, diameter / 2, Paint()..color = color);
        final painter = TextPainter(
          text: TextSpan(
            text: label,
            style: TextStyle(
              color: Colors.white,
              fontSize: strokeWidth,
              fontWeight: FontWeight.w700,
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        painter.paint(
          canvas,
          center - Offset(painter.width / 2, painter.height / 2),
        );
      case AnnotationTool.watermark:
        final rect = Rect.fromPoints(start, end);
        if (rect.isEmpty || (text ?? '').isEmpty) return;
        final painter = TextPainter(
          text: TextSpan(
            text: text,
            style: TextStyle(
              color: color.withValues(alpha: .4),
              fontSize: strokeWidth,
              fontFamily: 'Segoe UI',
              height: 1.32,
            ),
          ),
          textDirection: TextDirection.ltr,
        )..layout();
        canvas.save();
        canvas.clipRect(rect);
        for (double y = rect.top; y < rect.bottom; y += painter.height + 22) {
          for (double x = rect.left; x < rect.right; x += painter.width + 14) {
            painter.paint(canvas, Offset(x, y));
          }
        }
        canvas.restore();
      case AnnotationTool.mosaic:
      case AnnotationTool.spotlight:
      case AnnotationTool.smartErase:
        canvas.drawRect(
          Rect.fromPoints(start, end),
          brush..color = const Color(0xff1688f0),
        );
      case AnnotationTool.eraser:
        canvas.drawCircle(
          end,
          parameter > 0 ? parameter : 16,
          brush..color = const Color(0xff1688f0),
        );
    }
  }

  void _drawText(
    Canvas canvas,
    String value,
    Offset at,
    double size,
    Color ink, {
    bool emoji = false,
  }) {
    if (value.isEmpty) return;
    final painter = TextPainter(
      text: TextSpan(
        text: value,
        style: TextStyle(
          fontSize: size,
          color: ink,
          fontFamily: emoji ? 'Segoe UI Emoji' : 'Segoe UI',
          height: 1.32,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    painter.paint(canvas, at);
  }
}
