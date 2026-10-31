import 'dart:typed_data';

import 'package:flutter/services.dart';

import 'app_log.dart';
import 'ocr_document.dart';

class OcrBridge {
  static const _channel = MethodChannel('snappin/ocr');

  static Future<OcrDocument> recognizeLayout(Uint8List image) async {
    final watch = Stopwatch()..start();
    try {
      final data = await _channel.invokeMapMethod<dynamic, dynamic>(
        'recognizeLayout',
        image,
      );
      if (data == null) return OcrDocument(text: await recognize(image));
      final document = OcrDocument.fromJson(data);
      AppLog.write(
        'ocr-layout-ready',
        '${document.width}x${document.height} lines=${document.lines.length} ${watch.elapsedMilliseconds}ms',
      );
      return document;
    } on MissingPluginException {
      return OcrDocument(text: await recognize(image));
    } catch (error) {
      AppLog.write(
        'ocr-layout-failed',
        '${watch.elapsedMilliseconds}ms $error',
      );
      rethrow;
    }
  }

  static Future<String> recognize(Uint8List image) async {
    final watch = Stopwatch()..start();
    // The original image stays untouched; native OCR resizes only its input.
    final dimensions = image.length >= 24 && image[0] == 137 && image[1] == 80
        ? '${ByteData.sublistView(image).getUint32(16)}x${ByteData.sublistView(image).getUint32(20)}'
        : 'unknown';
    AppLog.write('ocr-local-start', 'source=$dimensions bytes=${image.length}');
    try {
      final text =
          await _channel.invokeMethod<String>('recognizeText', image) ?? '';
      AppLog.write(
        'ocr-local-complete',
        'characters=${text.length} ${watch.elapsedMilliseconds}ms',
      );
      return text;
    } catch (error) {
      AppLog.write('ocr-local-failed', '${watch.elapsedMilliseconds}ms $error');
      rethrow;
    }
  }
}
