import 'dart:io';
import 'dart:typed_data';

/// OCR owns a snapshot, never the source file being edited in another window.
class OcrImageStore {
  static Future<File> create(Uint8List png) async {
    final directory = await Directory.systemTemp.createTemp('snappin-ocr-');
    final file = File('${directory.path}/selection.png');
    try {
      return await file.writeAsBytes(png, flush: true);
    } catch (_) {
      await deleteOwned(file.path);
      rethrow;
    }
  }

  static Future<void> deleteOwned(String path) async {
    final file = File(path).absolute;
    final directory = file.parent;
    final parts = directory.uri.pathSegments.where((part) => part.isNotEmpty);
    final name = parts.isEmpty ? '' : parts.last;
    if (!RegExp(r'^snappin-ocr-[a-zA-Z0-9_-]+$').hasMatch(name) ||
        file.uri.pathSegments.last != 'selection.png') {
      return;
    }
    try {
      final temp = await Directory.systemTemp.resolveSymbolicLinks();
      final parent = await directory.parent.resolveSymbolicLinks();
      if ((Platform.isWindows ? parent.toLowerCase() : parent) !=
          (Platform.isWindows ? temp.toLowerCase() : temp)) {
        return;
      }
      if (await FileSystemEntity.type(directory.path, followLinks: false) !=
          FileSystemEntityType.directory) {
        return;
      }
      if (await FileSystemEntity.type(file.path, followLinks: false) ==
          FileSystemEntityType.file) {
        await file.delete();
      }
      // Never recursively delete a directory supplied by a result window.
      // Leave unexpected additional files untouched.
      await directory.delete();
    } on FileSystemException {
      // Already removed or contains additional files: keep the remaining data.
    }
  }
}
