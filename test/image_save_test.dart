import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/image_save.dart';

void main() {
  test('save format follows the selected file extension', () {
    final source = img.Image(width: 16, height: 16);
    img.fill(source, color: img.ColorRgb8(220, 40, 30));
    final png = Uint8List.fromList(img.encodePng(source));

    expect(identical(encodeImageForPath(png, 'capture.png'), png), isTrue);
    final jpeg = encodeImageForPath(png, 'capture.JPEG');
    expect(jpeg.sublist(0, 2), [0xff, 0xd8]);
    final decoded = img.decodeJpg(jpeg);
    expect(decoded?.width, 16);
    expect(decoded?.height, 16);
    expect(decoded!.getPixel(8, 8).r, closeTo(220, 5));
  });
}
