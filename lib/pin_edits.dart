import 'dart:typed_data';

import 'package:flutter/foundation.dart';
import 'package:image/image.dart' as img;

class PinEdits {
  const PinEdits({
    this.quarterTurns = 0,
    this.flipHorizontal = false,
    this.flipVertical = false,
    this.grayscale = false,
    this.invert = false,
    this.alpha = 1,
    this.brightness = 1,
  });

  final int quarterTurns;
  final bool flipHorizontal;
  final bool flipVertical;
  final bool grayscale;
  final bool invert;
  final double alpha;
  final double brightness;

  PinEdits copyWith({
    int? quarterTurns,
    bool? flipHorizontal,
    bool? flipVertical,
    bool? grayscale,
    bool? invert,
    double? alpha,
    double? brightness,
  }) => PinEdits(
    quarterTurns: quarterTurns ?? this.quarterTurns,
    flipHorizontal: flipHorizontal ?? this.flipHorizontal,
    flipVertical: flipVertical ?? this.flipVertical,
    grayscale: grayscale ?? this.grayscale,
    invert: invert ?? this.invert,
    alpha: alpha ?? this.alpha,
    brightness: brightness ?? this.brightness,
  );
}

Future<Uint8List> renderPinEdits(Uint8List source, PinEdits edits) =>
    compute(_render, (source, edits));

Uint8List _render((Uint8List, PinEdits) request) {
  final (source, edits) = request;
  var image = img.decodeImage(source);
  if (image == null) throw FormatException('Unsupported image data');
  image = image.convert(numChannels: 4);
  final turns = edits.quarterTurns % 4;
  if (turns != 0) image = img.copyRotate(image, angle: turns * 90);
  if (edits.flipHorizontal) image = img.flipHorizontal(image);
  if (edits.flipVertical) image = img.flipVertical(image);
  if (edits.grayscale) image = img.grayscale(image);
  if (edits.invert) image = img.invert(image);
  if (edits.brightness != 1) {
    image = img.adjustColor(image, brightness: edits.brightness);
  }
  if (edits.alpha != 1) {
    for (final pixel in image) {
      pixel.a = pixel.a * edits.alpha;
    }
  }
  return Uint8List.fromList(img.encodePng(image));
}
