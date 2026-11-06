import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/screenshot_archive.dart';

void main() {
  late Directory root;
  final png = Uint8List.fromList(
    img.encodePng(img.Image(width: 837, height: 1100)),
  );
  setUp(() async {
    root = await Directory.systemTemp.createTemp('snappin-archive-test-');
  });
  tearDown(() async {
    await root.delete(recursive: true);
  });

  test('archives retain original bytes, use the existing filename and roll over by local day', () async {
    var day = DateTime(2026, 11, 7, 23, 59, 59);
    final archive = ScreenshotArchive(root: root, now: () => day);
    final first = await archive.savePng(png);
    expect(
      first.parent.uri.pathSegments.where((s) => s.isNotEmpty).last,
      '2026-11-07',
    );
    expect(
      first.uri.pathSegments.last,
      matches(r'^snappin_20261107235959_\d{4}\.png$'),
    );
    expect(await first.readAsBytes(), png);
    day = DateTime(2026, 11, 8);
    final next = await archive.saveFile(first);
    expect(
      next.parent.uri.pathSegments.where((s) => s.isNotEmpty).last,
      '2026-11-08',
    );
    expect(
      next.uri.pathSegments.last,
      matches(r'^snappin_20261108000000_\d{4}\.png$'),
    );
    expect(await next.readAsBytes(), png);
    expect(img.decodePng(await next.readAsBytes())!.width, 837);
  });

  test('history lists complete original screenshots in date order and ignores unrelated files', () async {
    var time = DateTime(2026, 11, 6);
    final archive = ScreenshotArchive(root: root, now: () => time);
    final first = await archive.savePng(png);
    time = DateTime(2026, 11, 7);
    final second = await archive.savePng(png);
    await File('${first.parent.path}/other.png').writeAsBytes(png);
    await File('${first.parent.path}/snappin_20261106000000_9999.png')
        .writeAsBytes([1, 2]);
    final entries = await archive.list();
    expect(entries.map((e) => e['path']), [second.path, first.path]);
    expect(entries.map((e) => e['day']), ['2026-11-07', '2026-11-06']);
    expect(
      entries.every((e) => e['width'] == 837 && e['height'] == 1100),
      isTrue,
    );
  });

  test('filename collisions preserve existing screenshots with the same naming format', () async {
    final names = [1, 1, 2].iterator;
    final archive = ScreenshotArchive(
      root: root,
      now: () => DateTime(2026, 11, 7),
      filename: (_) {
        names.moveNext();
        return 'snappin_20261107000000_${names.current.toString().padLeft(4, '0')}.png';
      },
    );
    final first = await archive.savePng(png);
    final second = await archive.savePng(png);
    expect(first.path, isNot(second.path));
    expect(await first.readAsBytes(), png);
    expect(await second.readAsBytes(), png);
  });

  test(
    'failed archive removes its incomplete output and remains usable',
    () async {
      final archive = ScreenshotArchive(root: root);
      await expectLater(
        archive.savePng(Uint8List.fromList([1, 2, 3])),
        throwsStateError,
      );
      expect(
        await root.list(recursive: true).where((item) => item is File).toList(),
        isEmpty,
      );
      final complete = await archive.savePng(png);
      expect(await complete.readAsBytes(), png);
    },
  );
}
