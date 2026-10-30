import 'dart:io';
import 'dart:math';
import 'dart:typed_data';

import 'package:file_selector/file_selector.dart';
import 'package:image/image.dart' as img;

String defaultSaveName() {
  final now = DateTime.now();
  final stamp = [
    now.year.toString().padLeft(4, '0'),
    now.month.toString().padLeft(2, '0'),
    now.day.toString().padLeft(2, '0'),
    now.hour.toString().padLeft(2, '0'),
    now.minute.toString().padLeft(2, '0'),
    now.second.toString().padLeft(2, '0'),
  ].join();
  final suffix = Random.secure().nextInt(10000).toString().padLeft(4, '0');
  return 'snappin_${stamp}_$suffix.png';
}

Future<bool> saveImage(Uint8List png) async {
  final location = await getSaveLocation(
    suggestedName: defaultSaveName(),
    acceptedTypeGroups: const [
      XTypeGroup(label: 'PNG 图片', extensions: ['png']),
      XTypeGroup(label: 'JPEG 图片', extensions: ['jpg', 'jpeg']),
    ],
  );
  if (location == null) return false;
  await File(location.path)
      .writeAsBytes(encodeImageForPath(png, location.path));
  return true;
}

Uint8List encodeImageForPath(Uint8List png, String path) {
  final extension = path.toLowerCase();
  if (!extension.endsWith('.jpg') && !extension.endsWith('.jpeg')) return png;
  final image = img.decodePng(png);
  if (image == null) throw StateError('保存失败：PNG 图像无法解码');
  return Uint8List.fromList(img.encodeJpg(image, quality: 90));
}
