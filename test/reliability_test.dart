import 'dart:io';
import 'dart:typed_data';
import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/app_config.dart';
import 'package:snappin_flutter/app_log.dart';
import 'package:snappin_flutter/image_save.dart';
import 'package:snappin_flutter/owned_images.dart';
import 'package:snappin_flutter/png_rows.dart';

void main() {
  late Directory directory;
  setUp(() async {
    directory = await Directory.systemTemp.createTemp(
      'snappin-reliability-test-',
    );
  });
  tearDown(() async {
    AppLog.flush();
    await directory.delete(recursive: true);
  });

  test('queued config updates merge drag position and settings without stale overwrite', () async {
    final repository = ConfigRepository(File('${directory.path}/config.json'));
    await Future.wait([
      repository.update((c) => c.copyWith(ballCenter: const Offset(50, 60))),
      repository.update(
        (c) => c.copyWith(ballSize: 72, shortcutCapture: 'Ctrl+3'),
      ),
      repository.update((c) => c.copyWith(ballCenter: const Offset(90, 100))),
      repository.update((c) => c.copyWith(targetLang: 'English')),
    ]);
    final saved = await repository.load();
    expect(saved.ballCenter, const Offset(90, 100));
    expect(saved.ballSize, 72);
    expect(saved.shortcutCapture, 'Ctrl+3');
    expect(saved.targetLang, 'English');
    expect(await directory.list().where((f) => f is Directory).length, 0);
  });

  test(
    'malformed config recovers last good backup and can be saved again',
    () async {
      final file = File('${directory.path}/config.json');
      final repository = ConfigRepository(file);
      await repository.update((c) => c.copyWith(ballSize: 68));
      await repository.update((c) => c.copyWith(ballSize: 72));
      await file.writeAsString('{broken');
      expect((await repository.load()).ballSize, 68);
      await repository.update((c) => c.copyWith(targetLang: '日本語'));
      expect((await repository.load()).ballSize, 68);
      expect((await repository.load()).targetLang, '日本語');
    },
  );

  test(
    'failed config update does not poison queue or erase current settings',
    () async {
      final repository = ConfigRepository(
        File('${directory.path}/config.json'),
      );
      await repository.update((c) => c.copyWith(ballSize: 70));
      await expectLater(
        repository.update((_) => throw StateError('injected')),
        throwsStateError,
      );
      await repository.update((c) => c.copyWith(targetLang: 'English'));
      expect((await repository.load()).ballSize, 70);
    },
  );

  test(
    'background image save preserves original if JPEG encoding fails',
    () async {
      final target = File('${directory.path}/existing.jpg');
      await target.writeAsBytes([11, 22, 33]);
      await expectLater(
        writeImage(Uint8List.fromList([0]), target.path),
        throwsStateError,
      );
      expect(await target.readAsBytes(), [11, 22, 33]);
      final source = Uint8List.fromList(
        img.encodePng(img.Image(width: 20, height: 30)),
      );
      await writeImage(source, target.path);
      final decoded = img.decodeJpg(await target.readAsBytes());
      expect(decoded?.width, 20);
      expect(decoded?.height, 30);
    },
  );

  test(
    'streamed long PNG keeps width, alpha and all rows beyond 32000px',
    () async {
      final target = File('${directory.path}/long.png');
      const width = 17, height = 33001;
      Uint8List row(int y) => Uint8List.fromList(
        List.generate(width * 4, (i) => (y * 31 + i * 7) & 255),
      );
      await writePngRows(target, width, height, row);
      final image = img.decodePng(await target.readAsBytes())!;
      expect(image.width, width);
      expect(image.height, height);
      final bytes = image.getBytes(order: img.ChannelOrder.rgba);
      for (final y in [0, 1, 1023, 1024, 32000, 33000]) {
        expect(bytes.sublist(y * width * 4, (y + 1) * width * 4), row(y));
      }
    },
  );

  test(
    'streamed PNG write failure preserves previous file and removes staging',
    () async {
      final target = File('${directory.path}/long.png');
      await target.writeAsBytes([1, 2, 3]);
      await expectLater(
        writePngRows(target, 17, 5, (_) => throw StateError('injected')),
        throwsStateError,
      );
      expect(await target.readAsBytes(), [1, 2, 3]);
      expect(await directory.list().length, 1);
    },
  );

  test('result cleanup deletes only explicitly transferred images', () async {
    final owned = await OwnedImages.create();
    final image = File('${owned.path}/pin.png');
    final extra = File('${owned.path}/user-file.txt');
    final external = File('${directory.path}/source.png');
    await image.writeAsBytes([1]);
    await extra.writeAsBytes([2]);
    await external.writeAsBytes([3]);
    try {
      await OwnedImages.delete(owned.path, [image.path, external.path]);
      expect(await image.exists(), false);
      expect(await extra.readAsBytes(), [2]);
      expect(await external.readAsBytes(), [3]);
      await OwnedImages.delete(directory.path, [external.path]);
      expect(await external.exists(), true);
      await extra.delete();
      await OwnedImages.delete(owned.path, []);
      expect(await owned.exists(), false);
    } finally {
      if (await owned.exists()) await owned.delete(recursive: true);
    }
  });

  test('buffered log flushes critical failures and explicit shutdown without losing detail', () async {
    AppLog.initialize(logDirectory: directory);
    AppLog.write('long-manual-frame', 'height=33001 width=837');
    AppLog.write('capture-failed', 'injected');
    var log = await File(AppLog.path).readAsString();
    expect(log, contains('height=33001 width=837'));
    expect(log, contains('[capture-failed] injected'));
    AppLog.write('last-detail', 'retain');
    AppLog.flush();
    log = await File(AppLog.path).readAsString();
    expect(log, contains('[last-detail] retain'));
  });
}
