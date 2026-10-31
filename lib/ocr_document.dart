import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/material.dart';

class OcrTextLine {
  const OcrTextLine(this.text, this.bounds);
  final String text;
  final Rect bounds;
  Map<String, Object> toJson() => {
    'text': text,
    'x': bounds.left,
    'y': bounds.top,
    'width': bounds.width,
    'height': bounds.height,
  };
}

class OcrDocument {
  const OcrDocument({
    required this.text,
    this.width = 0,
    this.height = 0,
    this.lines = const [],
  });
  final String text;
  final double width, height;
  final List<OcrTextLine> lines;
  bool get hasLayout => width > 0 && height > 0 && lines.isNotEmpty;
  factory OcrDocument.fromJson(Map data) {
    final lines = <OcrTextLine>[];
    for (final item in data['lines'] as List? ?? const []) {
      final rect = Rect.fromLTWH(
        (item['x'] as num).toDouble(),
        (item['y'] as num).toDouble(),
        (item['width'] as num).toDouble(),
        (item['height'] as num).toDouble(),
      );
      if (rect.isFinite && rect.width > 0 && rect.height > 0) {
        lines.add(OcrTextLine(item['text'] as String, rect));
      }
    }
    lines.sort((a, b) {
      final order = a.bounds.top.compareTo(b.bounds.top);
      return order != 0 ? order : a.bounds.left.compareTo(b.bounds.left);
    });
    return OcrDocument(
      text: lines.isEmpty
          ? data['text'] as String? ?? ''
          : lines.map((line) => line.text).join('\n'),
      width: (data['width'] as num? ?? 0).toDouble(),
      height: (data['height'] as num? ?? 0).toDouble(),
      lines: lines,
    );
  }
  static OcrDocument decode(String value) =>
      OcrDocument.fromJson(jsonDecode(value) as Map);
  Map<String, Object> toJson() => {
    'text': text,
    'width': width,
    'height': height,
    'lines': lines.map((line) => line.toJson()).toList(),
  };
}

/// Keeps the source coordinates, including gaps and separate text columns.
class OcrLayoutView extends StatelessWidget {
  const OcrLayoutView({super.key, required this.document});
  final OcrDocument document;
  @override
  Widget build(BuildContext context) {
    final heights = document.lines.map((line) => line.bounds.height).toList()
      ..sort();
    final zoom = heights.isEmpty
        ? 1.0
        : math.max(1.0, 14 / heights[heights.length ~/ 2]);
    return SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      child: SingleChildScrollView(
        child: SelectionArea(
          child: SizedBox(
            width: document.width * zoom + 36,
            height: document.height * zoom + 28,
            child: Stack(
              children: [
                for (final line in document.lines)
                  Positioned(
                    left: 18 + line.bounds.left * zoom,
                    top: 14 + line.bounds.top * zoom,
                    child: Text(
                      line.text,
                      softWrap: false,
                      style: TextStyle(
                        fontSize: line.bounds.height * zoom * .95,
                        height: 1,
                        color: const Color(0xff1f2329),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
