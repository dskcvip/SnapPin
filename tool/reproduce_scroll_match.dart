import 'dart:io';

import 'package:image/image.dart' as img;
import 'package:snappin_flutter/long_stitch.dart';

void main(List<String> args) {
  final page = img.decodePng(File(args.first).readAsBytesSync())!;
  const height = 976;
  final last = page.height - height;
  final positions = [
    8000,
    for (var y = 8730; y < last; y += 730) y,
    last,
    for (var y = last - 710; y > 0; y -= 710) y,
    0,
  ];
  img.Image frame(int top) {
    final frame = img.copyCrop(
      page,
      x: 0,
      y: top,
      width: page.width,
      height: height,
    );
    img.fillRect(
      frame,
      x1: 705,
      y1: 80,
      x2: 842,
      y2: 200,
      color: img.ColorRgb8(255, 255, 255),
    );
    img.drawString(
      frame,
      'STATIC POPUP',
      font: img.arial14,
      x: 710,
      y: 100,
      color: img.ColorRgb8(0, 0, 0),
    );
    return frame;
  }

  final session = LongStitchSession.startRgba(
    frame(positions.first).getBytes(order: img.ChannelOrder.rgba),
    page.width,
    height,
  );
  var expectedMin = positions.first, expectedMax = positions.first;
  for (final top in positions.skip(1)) {
    final step = session.pushRgba(
      frame(top).getBytes(order: img.ChannelOrder.rgba),
      page.width,
      height,
    );
    if (top < expectedMin) expectedMin = top;
    if (top > expectedMax) expectedMax = top;
    final expected = expectedMax - expectedMin + height;
    if (session.height != expected) {
      stdout.writeln(
        'BAD top=$top kind=${step.kind.name} direction=${step.direction} height=${session.height} expected=$expected',
      );
    }
  }
  stdout.writeln(
    'actual=${session.height} expected=${page.height} width=${session.width}',
  );
  session.dispose();
}
