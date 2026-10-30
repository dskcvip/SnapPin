import 'dart:io';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:image/image.dart' as img;

class MosaicPreview {
  const MosaicPreview(this.width, this.height, this.pixels);
  final int width, height;
  final Uint8List pixels;

  static Future<MosaicPreview> load(String path) async =>
      compute(_decode, await File(path).readAsBytes());

  void paint(Canvas canvas, Rect region, Size sourceSize, double blockSize) {
    final rect = region.intersect(Offset.zero & sourceSize);
    final block = blockSize.clamp(2.0, 128.0);
    final brush = Paint()..isAntiAlias = false;
    for (var y = rect.top; y < rect.bottom; y += block) {
      for (var x = rect.left; x < rect.right; x += block) {
        final cell = Rect.fromLTRB(
          x,
          y,
          (x + block).clamp(0, rect.right),
          (y + block).clamp(0, rect.bottom),
        );
        final x0 = (cell.left * width / sourceSize.width).floor().clamp(
          0,
          width - 1,
        );
        final y0 = (cell.top * height / sourceSize.height).floor().clamp(
          0,
          height - 1,
        );
        final x1 = (cell.right * width / sourceSize.width).ceil().clamp(
          x0 + 1,
          width,
        );
        final y1 = (cell.bottom * height / sourceSize.height).ceil().clamp(
          y0 + 1,
          height,
        );
        var r = 0, g = 0, b = 0, a = 0, count = 0;
        for (var py = y0; py < y1; py++) {
          for (var px = x0; px < x1; px++) {
            final offset = (py * width + px) * 4;
            r += pixels[offset];
            g += pixels[offset + 1];
            b += pixels[offset + 2];
            a += pixels[offset + 3];
            count++;
          }
        }
        canvas.drawRect(
          cell,
          brush
            ..color = Color.fromARGB(
              a ~/ count,
              r ~/ count,
              g ~/ count,
              b ~/ count,
            ),
        );
      }
    }
  }
}

MosaicPreview _decode(Uint8List bytes) {
  var image = img.decodePng(bytes);
  if (image == null) throw StateError('图片无法解码');
  if (image.width > 4096 || image.height > 4096) {
    image = img.copyResize(
      image,
      width: image.width >= image.height ? 4096 : null,
      height: image.height > image.width ? 4096 : null,
    );
  }
  return MosaicPreview(
    image.width,
    image.height,
    image.getBytes(order: img.ChannelOrder.rgba),
  );
}
