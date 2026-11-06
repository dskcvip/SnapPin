import 'dart:io';
import 'dart:isolate';
import 'dart:math';
import 'dart:typed_data';

import 'package:file_selector/file_selector.dart';
import 'package:image/image.dart' as img;

import 'atomic_file.dart';
import 'app_log.dart';

String defaultSaveName({DateTime? time}) {
  final now = time ?? DateTime.now();
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

Future<bool> saveImage(Uint8List png, {void Function(bool)? onWriting}) async {
  final location = await getSaveLocation(
    suggestedName: defaultSaveName(),
    acceptedTypeGroups: const [
      XTypeGroup(label: 'PNG 图片', extensions: ['png']),
      XTypeGroup(label: 'JPEG 图片', extensions: ['jpg', 'jpeg']),
    ],
  );
  if (location == null) return false;
  onWriting?.call(true);
  final elapsed = Stopwatch()..start();
  AppLog.write('image-save-start', 'bytes=${png.length}');
  try {
    await writeImage(png, location.path);
    AppLog.write(
      'image-save-finished',
      'elapsed=${elapsed.elapsedMilliseconds}ms',
    );
    return true;
  } catch (error) {
    AppLog.write('image-save-failed', 'type=${error.runtimeType}');
    rethrow;
  } finally {
    onWriting?.call(false);
  }
}

/// Encoding and disk staging run independently of the rendering isolate.
Future<void> writeImage(Uint8List png, String path) => Isolate.run(() async {
  await atomicWrite(File(path), encodeImageForPath(png, path));
});

Uint8List encodeImageForPath(Uint8List png, String path) {
  final extension = path.toLowerCase();
  if (!extension.endsWith('.jpg') && !extension.endsWith('.jpeg')) return png;
  final image = img.decodePng(png);
  if (image == null) throw StateError('保存失败：PNG 图像无法解码');
  return Uint8List.fromList(img.encodeJpg(image, quality: 90));
}
