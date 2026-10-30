import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/ocr_image_store.dart';

void main() {
  test(
    'OCR snapshot survives source edits and cleans only its owned file',
    () async {
      final sourceDirectory = await Directory.systemTemp.createTemp(
        'snappin-pin-',
      );
      final source = File('${sourceDirectory.path}/source.png');
      await source.writeAsBytes([1, 2, 3]);
      final snapshot = await OcrImageStore.create(await source.readAsBytes());
      try {
        await source.writeAsBytes([4, 5, 6]);
        expect(await snapshot.readAsBytes(), [1, 2, 3]);
        await OcrImageStore.deleteOwned(source.path);
        expect(await source.readAsBytes(), [4, 5, 6]);
        await OcrImageStore.deleteOwned(snapshot.path);
        expect(await snapshot.exists(), isFalse);
        expect(await source.exists(), isTrue);
      } finally {
        await OcrImageStore.deleteOwned(snapshot.path);
        await sourceDirectory.delete(recursive: true);
      }
    },
  );

  test(
    'OCR cleanup preserves unrelated files even in its own temp directory',
    () async {
      final snapshot = await OcrImageStore.create(Uint8List.fromList([1]));
      final other = File('${snapshot.parent.path}/other.png');
      await other.writeAsBytes([2]);
      try {
        await OcrImageStore.deleteOwned(snapshot.path);
        expect(await other.exists(), isTrue);
        expect(await snapshot.exists(), isFalse);
      } finally {
        await other.delete();
        await snapshot.parent.delete();
      }
    },
  );

  test(
    'OCR cleanup ignores same-named images outside its own temp root',
    () async {
      final parent = await Directory.systemTemp.createTemp('snappin-external-');
      final nested = await Directory('${parent.path}/snappin-ocr-test')
          .create();
      final source = File('${nested.path}/selection.png');
      await source.writeAsBytes([3]);
      try {
        await OcrImageStore.deleteOwned(source.path);
        expect(await source.exists(), isTrue);
      } finally {
        await parent.delete(recursive: true);
      }
    },
  );
}
