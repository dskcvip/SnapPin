import 'dart:typed_data';
import 'dart:ui';

import 'package:flutter/foundation.dart';
import 'package:image/image.dart' as img;
import 'package:zxing_lib/common.dart';
import 'package:zxing_lib/multi.dart';
import 'package:zxing_lib/zxing.dart';

class DecodedCode {
  const DecodedCode(this.format, this.text, this.points);

  final String format;
  final String text;
  final List<Offset> points;
}

Future<List<DecodedCode>> scanCodes(Uint8List bytes) =>
    compute(_scanCodes, bytes);

List<DecodedCode> _scanCodes(Uint8List bytes) {
  final image = img.decodeImage(bytes);
  if (image == null) return const [];
  final pixels = image
      .convert(numChannels: 4)
      .getBytes(order: img.ChannelOrder.bgra)
      .buffer
      .asInt32List();
  final bitmap = BinaryBitmap(
    HybridBinarizer(RGBLuminanceSource(image.width, image.height, pixels)),
  );
  final results = <Result>[];
  try {
    results.addAll(QRCodeMultiReader().decodeMultiple(bitmap));
  } on ReaderException {
    // The selection may contain only one-dimensional barcodes.
  }
  try {
    final reader = GenericMultipleBarcodeReader(MultiFormatReader());
    for (final result in reader.decodeMultiple(bitmap)) {
      if (!results.any((existing) =>
          existing.barcodeFormat == result.barcodeFormat &&
          existing.text == result.text)) {
        results.add(result);
      }
    }
  } on ReaderException {
    // Empty selections are a normal result.
  }
  return [
    for (final result in results)
      DecodedCode(
        result.barcodeFormat.name.toUpperCase(),
        result.text,
        [
          for (final point in result.resultPoints ?? const <ResultPoint?>[])
            if (point != null) Offset(point.x, point.y),
        ],
      ),
  ];
}
