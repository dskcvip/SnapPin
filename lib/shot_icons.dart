import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

class ShotIcon extends StatelessWidget {
  const ShotIcon(
    this.name, {
    super.key,
    this.size = 22,
    this.color = const Color(0xff4c4c4c),
  });

  final String name;
  final double size;
  final Color color;

  static const _scales = <String, double>{
    'ellipse': 1.2,
    'sticker': 1.2,
    'pen': 1.2,
    'badge': 1.2,
    'eraser': 1.2,
    'smart-erase': 1.05,
    'translate': 1.22,
    'ocr': 1.26,
    'long': 1.28,
    'undo': 1.1,
    'redo': 1.1,
    'save': 1.28,
    'pin': 1.15,
    'close': 1.17,
    'check': 1.4,
    'qr': 1.1,
  };

  @override
  Widget build(BuildContext context) {
    final scale = _scales[name] ?? 1;
    final drawingSize = size * scale;
    final inset = (size - drawingSize) / 2;
    return SizedBox.square(
      dimension: size,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned(
            left: inset,
            top: inset,
            width: drawingSize,
            height: drawingSize,
            child: SvgPicture.asset(
              'assets/icons/shot/$name.svg',
              width: drawingSize,
              height: drawingSize,
              theme: SvgTheme(currentColor: color),
            ),
          ),
          if (name == 'translate' || name == 'ocr')
            Positioned(
              left: inset,
              top: inset,
              width: drawingSize,
              height: drawingSize,
              child: CustomPaint(
                painter: _IconLetterPainter(
                  x: name == 'translate' ? 7.95 : 12,
                  baseline: name == 'translate' ? 16.6 : 15,
                  fontSize: name == 'translate' ? 8.1 : 8.7,
                  color: color,
                ),
              ),
            ),
        ],
      ),
    );
  }
}

class _IconLetterPainter extends CustomPainter {
  const _IconLetterPainter({
    required this.x,
    required this.baseline,
    required this.fontSize,
    required this.color,
  });

  final double x, baseline, fontSize;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final scale = size.width / 24;
    final text = TextPainter(
      text: TextSpan(
        text: '文',
        style: TextStyle(
          fontSize: fontSize * scale,
          fontFamily: 'Microsoft YaHei',
          color: color,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    final ascent = text.computeDistanceToActualBaseline(TextBaseline.alphabetic);
    text.paint(
      canvas,
      Offset(x * scale - text.width / 2, baseline * scale - ascent),
    );
  }

  @override
  bool shouldRepaint(covariant _IconLetterPainter old) =>
      old.x != x ||
      old.baseline != baseline ||
      old.fontSize != fontSize ||
      old.color != color;
}
