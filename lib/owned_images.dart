import 'dart:io';

import 'app_log.dart';

/// A directory explicitly created and transferred to one result window.
/// Cleanup never scans the system temp folder or follows directory symlinks.
class OwnedImages {
  static const _marker = '.snappin-images';

  static Future<Directory> create() async {
    final directory = await Directory.systemTemp.createTemp('snappin-images-');
    await File('${directory.path}/$_marker').writeAsString('SnapPin images v1');
    return directory;
  }

  static Future<void> delete(String path, Iterable<String> files) async {
    final directory = Directory(path).absolute;
    try {
      if (await FileSystemEntity.type(directory.path, followLinks: false) !=
          FileSystemEntityType.directory)
        return;
      final temp = await Directory.systemTemp.resolveSymbolicLinks();
      final parent = await directory.parent.resolveSymbolicLinks();
      bool same(String a, String b) => Platform.isWindows
          ? a.replaceAll('\\', '/').toLowerCase() ==
                b.replaceAll('\\', '/').toLowerCase()
          : a == b;
      if (!same(temp, parent) ||
          !directory.uri.pathSegments
              .where((p) => p.isNotEmpty)
              .last
              .startsWith('snappin-images-'))
        return;
      final marker = File('${directory.path}/$_marker');
      if (await FileSystemEntity.type(marker.path, followLinks: false) !=
              FileSystemEntityType.file ||
          await marker.readAsString() != 'SnapPin images v1')
        return;
      for (final path in files.toSet()) {
        final file = File(path).absolute;
        if (!same(file.parent.path, directory.path)) continue;
        if (await FileSystemEntity.type(file.path, followLinks: false) ==
            FileSystemEntityType.file) {
          await file.delete();
        }
      }
      final remaining = await directory.list().toList();
      if (remaining.length == 1 && same(remaining.single.path, marker.path)) {
        await marker.delete();
        await directory.delete();
      }
    } on FileSystemException catch (error) {
      AppLog.write('image-cleanup-failed', error.osError?.errorCode);
    }
  }
}
