import 'dart:typed_data';

import 'package:flutter/services.dart';

class OcrBridge {
  static const _channel = MethodChannel('snappin/ocr');

  static Future<String> recognize(Uint8List image) async =>
      await _channel.invokeMethod<String>('recognizeText', image) ?? '';
}
