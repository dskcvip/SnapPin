import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:image/image.dart' as img;

import 'annotation.dart';
import 'capture_image.dart';

typedef _Tile = ({
  int x,
  int y,
  int insetX,
  int insetY,
  int width,
  int height,
  Uint8List png,
});

// Never upload an entire long image to the GPU: image codecs may silently
// reduce it to the maximum texture size. Only individual tiles use dart:ui.
Future<Uint8List> renderAnnotatedImage(
  Uint8List source,
  ui.Rect crop,
  List<Annotation> marks,
) async {
  final parts = await compute(_split, (
    source,
    crop.left.round(),
    crop.top.round(),
    crop.width.round(),
    crop.height.round(),
  ));
  final rendered = <_Tile>[];
  for (final tile in parts.$5) {
    final codec = await ui.instantiateImageCodec(tile.png);
    final image = (await codec.getNextFrame()).image;
    try {
      final origin = ui.Offset(
        (parts.$3 + tile.x - tile.insetX).toDouble(),
        (parts.$4 + tile.y - tile.insetY).toDouble(),
      );
      final bytes = await cropSelection(
        image,
        ui.Rect.fromLTWH(0, 0, image.width.toDouble(), image.height.toDouble()),
        ui.Size(image.width.toDouble(), image.height.toDouble()),
        [
          for (final mark in marks)
            Annotation(
              tool: mark.tool,
              points: [for (final point in mark.points) point - origin],
              color: mark.color,
              strokeWidth: mark.strokeWidth,
              text: mark.text,
              number: mark.number,
              parameter: mark.parameter,
              stickerPicture: mark.stickerPicture,
              stickerPictureSize: mark.stickerPictureSize,
            ),
        ],
      );
      rendered.add((
        x: tile.x,
        y: tile.y,
        insetX: tile.insetX,
        insetY: tile.insetY,
        width: tile.width,
        height: tile.height,
        png: bytes,
      ));
    } finally {
      image.dispose();
      codec.dispose();
    }
  }
  return compute(_join, (parts.$1, parts.$2, rendered));
}

(int, int, int, int, List<_Tile>) _split((Uint8List, int, int, int, int) args) {
  final source = img.decodePng(args.$1);
  if (source == null) throw StateError('图片无法解码');
  final left = args.$2.clamp(0, source.width - 1);
  final top = args.$3.clamp(0, source.height - 1);
  final width = args.$4.clamp(1, source.width - left);
  final height = args.$5.clamp(1, source.height - top);
  final tiles = <_Tile>[];
  const side = 2048;
  const padding = 128;
  for (var y = 0; y < height; y += side) {
    for (var x = 0; x < width; x += side) {
      final x0 = math.max(0, x - padding);
      final y0 = math.max(0, y - padding);
      final w = math.min(side, width - x);
      final h = math.min(side, height - y);
      final tile = img.copyCrop(
        source,
        x: left + x0,
        y: top + y0,
        width: math.min(width, x + w + padding) - x0,
        height: math.min(height, y + h + padding) - y0,
      );
      tiles.add((
        x: x,
        y: y,
        insetX: x - x0,
        insetY: y - y0,
        width: w,
        height: h,
        png: Uint8List.fromList(img.encodePng(tile)),
      ));
    }
  }
  return (width, height, left, top, tiles);
}

Uint8List _join((int, int, List<_Tile>) args) {
  final output = img.Image(width: args.$1, height: args.$2, numChannels: 4);
  for (final tile in args.$3) {
    final image = img.decodePng(tile.png)!;
    img.compositeImage(
      output,
      image,
      dstX: tile.x,
      dstY: tile.y,
      srcX: tile.insetX,
      srcY: tile.insetY,
      srcW: tile.width,
      srcH: tile.height,
      dstW: tile.width,
      dstH: tile.height,
      blend: img.BlendMode.direct,
    );
  }
  return Uint8List.fromList(img.encodePng(output));
}
