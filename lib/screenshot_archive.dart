import 'dart:io';
import 'dart:typed_data';

import 'app_log.dart';
import 'image_save.dart';

/// Permanent screenshots belong beside the installation, outside a signed
/// macOS app bundle. Temporary result-window cleanup must never delete them.
class ScreenshotArchive {
  ScreenshotArchive({
    Directory? root,
    DateTime Function()? now,
    String Function(DateTime)? filename,
  }) : root = root ?? installationDirectory(),
       _now = now ?? DateTime.now,
       _filename = filename ?? ((time) => defaultSaveName(time: time));

  final Directory root;
  final DateTime Function() _now;
  final String Function(DateTime) _filename;

  static Directory installationDirectory() {
    var directory = File(Platform.resolvedExecutable).parent;
    if (Platform.isMacOS && directory.path.endsWith('/Contents/MacOS')) {
      directory = directory.parent.parent.parent;
    }
    return Directory('${directory.path}${Platform.pathSeparator}截图');
  }

  Future<void> prepare() async {
    await root.create(recursive: true);
  }

  /// Read headers only; listing never decodes full-resolution long screenshots.
  Future<List<Map<String, dynamic>>> list() async {
    final entries = <Map<String, dynamic>>[];
    if (!await root.exists()) return entries;
    await for (final directory in root.list(followLinks: false)) {
      final day = directory.uri.pathSegments.where((s) => s.isNotEmpty).last;
      if (directory is! Directory ||
          !RegExp(r'^\d{4}-\d{2}-\d{2}$').hasMatch(day))
        continue;
      await for (final file in directory.list(followLinks: false)) {
        if (file is! File) continue;
        final name = file.uri.pathSegments.last;
        if (!RegExp(r'^snappin_\d{14}_\d{4}\.png$').hasMatch(name)) continue;
        try {
          final handle = await file.open();
          late Uint8List header;
          try {
            header = await handle.read(24);
          } finally {
            await handle.close();
          }
          if (header.length != 24 || header[0] != 137 || header[1] != 80)
            continue;
          final data = ByteData.sublistView(header);
          final width = data.getUint32(16), height = data.getUint32(20);
          if (width == 0 || height == 0) continue;
          entries.add({
            'path': file.path,
            'day': day,
            'name': name,
            'width': width,
            'height': height,
          });
        } on FileSystemException catch (error) {
          AppLog.write('archive-list-file-skipped', '$name $error');
        }
      }
    }
    entries.sort((a, b) {
      final day = (b['day'] as String).compareTo(a['day'] as String);
      return day != 0
          ? day
          : (b['name'] as String).compareTo(a['name'] as String);
    });
    return entries;
  }

  Future<File> savePng(Uint8List bytes) => _save(Stream.value(bytes));

  Future<File> saveFile(File source) => _save(source.openRead());

  Future<File> _save(Stream<List<int>> source) async {
    final now = _now();
    String two(int value) => value.toString().padLeft(2, '0');
    final day =
        '${now.year.toString().padLeft(4, '0')}-${two(now.month)}-${two(now.day)}';
    final directory = await Directory(
      '${root.path}${Platform.pathSeparator}$day',
    ).create(recursive: true);
    File? reserved;
    for (var attempt = 0; attempt < 64; attempt++) {
      final name = _filename(now);
      if (!RegExp(r'^snappin_\d{14}_\d{4}\.png$').hasMatch(name)) {
        throw StateError('截图文件名不符合既定命名规则');
      }
      final candidate = File('${directory.path}${Platform.pathSeparator}$name');
      try {
        await candidate.create(exclusive: true);
        reserved = candidate;
        break;
      } on FileSystemException {
        if (!await candidate.exists()) rethrow;
      }
    }
    if (reserved == null) throw StateError('无法分配截图文件名');
    Directory? staging;
    File? temporary;
    try {
      staging = await directory.createTemp('.snappin-save-');
      temporary = File('${staging.path}${Platform.pathSeparator}image.png');
      final sink = temporary.openWrite();
      try {
        await sink.addStream(source);
        await sink.flush();
      } finally {
        await sink.close();
      }
      final header = await temporary.open();
      try {
        final signature = await header.read(8);
        const png = [137, 80, 78, 71, 13, 10, 26, 10];
        if (signature.length != 8 ||
            List.generate(8, (i) => signature[i] == png[i]).contains(false)) {
          throw StateError('截图 PNG 数据无效');
        }
      } finally {
        await header.close();
      }
      await temporary.rename(reserved.path);
      AppLog.write(
        'screenshot-archived',
        'path=${reserved.path} bytes=${await reserved.length()}',
      );
      return reserved;
    } catch (error) {
      await reserved.delete();
      AppLog.write('screenshot-archive-failed', '$error root=${root.path}');
      rethrow;
    } finally {
      if (temporary != null && await temporary.exists())
        await temporary.delete();
      if (staging != null) await staging.delete();
    }
  }
}
