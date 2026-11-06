import 'dart:io';
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:image/image.dart' as img;
import 'package:snappin_flutter/long_stitch.dart';
import 'package:snappin_flutter/long_stitch_worker.dart';

Uint8List _frame(int top) {
  final image = img.Image(width: 160, height: 600);
  for (var y = 0; y < image.height; y++) {
    final row = top + y;
    final seed = (row * 1103515245 + 12345) & 0x7fffffff;
    for (var x = 0; x < image.width; x++) {
      image.setPixelRgb(
        x,
        y,
        ((seed >> 16) + x * 5) % 256,
        ((seed >> 9) + x * 3) % 256,
        ((seed >> 2) + x * 7) % 256,
      );
    }
  }
  return Uint8List.fromList(img.encodePng(image));
}

Uint8List _unrelatedFrame() {
  final image = img.Image(width: 160, height: 600);
  img.fill(image, color: img.ColorRgb8(255, 255, 255));
  return Uint8List.fromList(img.encodePng(image));
}

Uint8List _frameWithFixedHeader(int top) {
  final image = img.decodePng(_frame(top))!;
  for (var y = 0; y < 80; y++) {
    for (var x = 0; x < image.width; x++) {
      image.setPixelRgb(x, y, 20, 36, 58);
    }
  }
  return Uint8List.fromList(img.encodePng(image));
}

void main() {
  test('overlap joins remove repeated viewport corner artifacts on both edges without changing width', () async {
    Uint8List corners(int top) {
      final image = img.decodePng(_frame(top))!;
      for (final y0 in [0, image.height - 6]) {
        for (var y = y0; y < y0 + 6; y++) {
          for (var x = 0; x < 5; x++) {
            image.setPixelRgb(x, y, 0, 0, 0);
            image.setPixelRgb(image.width - x - 1, y, 0, 0, 0);
          }
        }
      }
      return Uint8List.fromList(img.encodePng(image));
    }

    final session = LongStitchSession.start(corners(500));
    for (final top in [700, 900, 1100, 900, 700, 500, 300, 100, 0]) {
      session.push(corners(top));
    }
    final directory = Directory.systemTemp.createTempSync('snappin-corners-');
    addTearDown(() => directory.deleteSync(recursive: true));
    final file = await session.savePng(File('${directory.path}/result.png'));
    final result = img.decodePng(await file.readAsBytes())!;
    expect(result.width, 160);
    expect(result.height, 1700);
    for (var y = 6; y < result.height - 6; y++) {
      for (final x in [0, 1, 158, 159]) {
        final pixel = result.getPixel(x, y);
        final seed = (y * 1103515245 + 12345) & 0x7fffffff;
        expect(
          [pixel.r, pixel.g, pixel.b],
          [
            ((seed >> 16) + x * 5) % 256,
            ((seed >> 9) + x * 3) % 256,
            ((seed >> 2) + x * 7) % 256,
          ],
          reason: 'edge x=$x y=$y',
        );
      }
    }
  });

  test('repeated browser blocks with a fixed tooltip retain every row on fast reversal', () async {
    final page = img.decodePng(
      File('test/fixtures/long-scroll-page.png').readAsBytesSync(),
    )!;
    const frameHeight = 976;
    final last = page.height - frameHeight;
    final positions = [
      8000,
      for (var top = 8730; top < last; top += 730) top,
      last,
      for (var top = last - 710; top > 0; top -= 710) top,
      0,
    ];
    img.Image frame(int top) {
      final image = img.copyCrop(
        page,
        x: 0,
        y: top,
        width: page.width,
        height: frameHeight,
      );
      img.fillRect(
        image,
        x1: 705,
        y1: 80,
        x2: 842,
        y2: 200,
        color: img.ColorRgb8(255, 255, 255),
      );
      img.drawString(
        image,
        'STATIC POPUP',
        font: img.arial14,
        x: 710,
        y: 100,
        color: img.ColorRgb8(0, 0, 0),
      );
      return image;
    }

    final first = frame(positions.first);
    final session = LongStitchSession.startRgba(
      first.getBytes(order: img.ChannelOrder.rgba),
      page.width,
      frameHeight,
    );
    final directory = await Directory.systemTemp.createTemp('snappin-blocks-');
    try {
      var minimum = positions.first, maximum = positions.first;
      for (var index = 1; index < positions.length; index++) {
        final top = positions[index];
        final image = frame(top);
        final step = index.isEven
            ? session.push(Uint8List.fromList(img.encodePng(image)))
            : session.pushRgba(
                image.getBytes(order: img.ChannelOrder.rgba),
                page.width,
                frameHeight,
              );
        if (top < minimum) minimum = top;
        if (top > maximum) maximum = top;
        expect(step.kind, isNot(LongStepKind.noMatch), reason: 'top=$top');
        expect(
          session.height,
          maximum - minimum + frameHeight,
          reason: 'top=$top',
        );
      }
      final file = await session.savePng(File('${directory.path}/full.png'));
      final output = img.decodePng(await file.readAsBytes())!;
      expect((output.width, output.height), (page.width, page.height));
      // Ignore the fixed popup itself; verify the source rows on either side
      // of every block boundary, including text and the unique barcode.
      for (var y = 0; y < page.height; y++) {
        for (final x in [4, 30, 130, 300, 590]) {
          final a = output.getPixel(x, y), b = page.getPixel(x, y);
          expect((a.r, a.g, a.b), (b.r, b.g, b.b), reason: 'x=$x y=$y');
        }
      }
    } finally {
      session.dispose();
      await directory.delete(recursive: true);
    }
  });

  test(
    'raw worker preserves fast scroll, reversal and detached recovery pixels',
    () async {
      final pixels = img
          .decodePng(_frame(600))!
          .getBytes(order: img.ChannelOrder.rgba);
      final worker = await LongStitchWorker.start(
        pixels,
        rawWidth: 160,
        rawHeight: 600,
      );
      final reference = LongStitchSession.start(_frame(600));
      final directory = await Directory.systemTemp.createTemp('snappin-raw-');
      pixels.fillRange(0, pixels.length, 0);
      try {
        for (final top in [
          1000,
          1400,
          1800,
          1400,
          1000,
          600,
          0,
          120,
          240,
          360,
          480,
          601,
          721,
        ]) {
          final png = _frame(top);
          final image = img.decodePng(png)!;
          final step = await worker.pushRgba(
            image.getBytes(order: img.ChannelOrder.rgba),
            160,
            600,
          );
          final expected = reference.push(png);
          expect(
            (step.kind, step.direction, step.appended, step.height),
            (
              expected.kind,
              expected.direction,
              expected.appended,
              expected.height,
            ),
            reason: 'top=$top',
          );
        }
        await expectLater(
          worker.pushRgba(Uint8List(8), 160, 600),
          throwsStateError,
        );
        final actual = await worker.savePng(File('${directory.path}/raw.png'));
        final expected = await reference.savePng(
          File('${directory.path}/png.png'),
        );
        expect(
          img.decodePng(await actual.readAsBytes())!.getBytes(),
          img.decodePng(await expected.readAsBytes())!.getBytes(),
        );
        expect(worker.width, 160);
      } finally {
        worker.dispose();
        reference.dispose();
        await directory.delete(recursive: true);
      }
    },
  );

  test('raw session owns input and retains original width', () {
    final initial = img
        .decodePng(_frame(0))!
        .getBytes(order: img.ChannelOrder.rgba);
    final session = LongStitchSession.startRgba(initial, 160, 600);
    initial.fillRange(0, initial.length, 0);
    try {
      final next = img
          .decodePng(_frame(400))!
          .getBytes(order: img.ChannelOrder.rgba);
      expect(session.pushRgba(next, 160, 600).kind, LongStepKind.appended);
      next.fillRange(0, next.length, 0);
      expect(session.push(_frame(400)).kind, LongStepKind.still);
      expect(session.width, 160);
      expect(session.height, 1000);
      expect(() => session.pushRgba(Uint8List(0), 160, 600), throwsStateError);
    } finally {
      session.dispose();
    }
  });

  test('raw frames match PNG behavior under fixed navigation during fast scrolling', () {
    final initial = img.decodePng(_frameWithFixedHeader(0))!;
    final raw = LongStitchSession.startRgba(
      initial.getBytes(order: img.ChannelOrder.rgba),
      160,
      600,
    );
    final png = LongStitchSession.start(_frameWithFixedHeader(0));
    try {
      for (final top in [400, 820, 1200, 1600, 1200, 820, 400, 0]) {
        final frame = _frameWithFixedHeader(top);
        final decoded = img.decodePng(frame)!;
        final actual = raw.pushRgba(
          decoded.getBytes(order: img.ChannelOrder.rgba),
          160,
          600,
        );
        final expected = png.push(frame);
        expect(
          (actual.kind, actual.height, actual.appended),
          (expected.kind, expected.height, expected.appended),
        );
      }
    } finally {
      raw.dispose();
      png.dispose();
    }
  });

  test('Dart stitch previews, appends, and exports', () async {
    final session = LongStitchSession.start(_frame(0));
    try {
      expect(session.width, 160);
      expect(session.height, 600);
      expect(img.decodePng(session.previewPng())?.height, greaterThan(0));
      final step = session.push(_frame(120));
      expect(step.kind, LongStepKind.appended);
      expect(session.height, greaterThan(600));
      final directory = await Directory.systemTemp.createTemp(
        'snappin-stitch-test-',
      );
      try {
        final output = await session.savePng(
          File('${directory.path}/stitched.png'),
        );
        expect(
          img.decodePng(await output.readAsBytes())?.height,
          session.height,
        );
      } finally {
        await directory.delete(recursive: true);
      }
    } finally {
      session.dispose();
    }
  });

  test(
    'manual scrolling keeps captured rows when rewinding or losing overlap',
    () {
      final session = LongStitchSession.start(_frame(0));
      try {
        expect(session.push(_frame(120)).kind, LongStepKind.appended);
        expect(session.push(_frame(240)).kind, LongStepKind.appended);
        final height = session.height;
        expect(session.push(_frame(120)).kind, LongStepKind.rewound);
        expect(session.height, height);
        expect(session.push(_unrelatedFrame()).kind, LongStepKind.noMatch);
        expect(session.height, height);
        expect(session.push(_frame(240)).kind, LongStepKind.rewound);
        expect(session.height, height);
        expect(session.push(_frame(360)).kind, LongStepKind.appended);
        expect(session.height, greaterThan(height));
      } finally {
        session.dispose();
      }
    },
  );

  test('manual scroll matches content beneath a fixed header', () {
    final session = LongStitchSession.start(_frameWithFixedHeader(0));
    try {
      final step = session.push(_frameWithFixedHeader(120));
      expect(step.kind, LongStepKind.appended);
      expect(step.appended, closeTo(120, 8));
    } finally {
      session.dispose();
    }
  });

  test('fast manual scroll recovers across changing overlap', () {
    final session = LongStitchSession.start(_frameWithFixedHeader(0));
    try {
      for (final top in [400, 820, 1200, 1600]) {
        final step = session.push(_frameWithFixedHeader(top));
        expect(step.kind, LongStepKind.appended, reason: 'top=$top');
      }
      expect(session.height, greaterThan(2000));
    } finally {
      session.dispose();
    }
  });

  test('preview shrinks while exported image keeps original pixels', () async {
    final session = LongStitchSession.start(_frame(0));
    try {
      for (var top = 120; top <= 2160; top += 120) {
        expect(session.push(_frame(top)).kind, LongStepKind.appended);
      }
      expect(session.width, 160);
      expect(session.height, 2760);
      expect(img.decodePng(session.previewPng())!.height, lessThan(2760));
      final directory = await Directory.systemTemp.createTemp(
        'snappin-long-preview-',
      );
      try {
        final output = await session.savePng(
          File('${directory.path}/full.png'),
        );
        final image = img.decodePng(await output.readAsBytes())!;
        expect(image.width, 160);
        expect(image.height, 2760);
        expect(
          image.getPixel(5, 2200).r.toInt(),
          img.decodePng(_frame(1800))!.getPixel(5, 400).r.toInt(),
        );
      } finally {
        await directory.delete(recursive: true);
      }
    } finally {
      session.dispose();
    }
  });

  test(
    'scrolling down then above the starting point prepends without duplicates',
    () async {
      final session = LongStitchSession.start(_frame(600));
      try {
        for (var top = 720; top <= 1800; top += 120) {
          expect(session.push(_frame(top)).kind, LongStepKind.appended);
        }
        final bottomHeight = session.height;
        for (var top = 1680; top >= 600; top -= 120) {
          expect(session.push(_frame(top)).kind, LongStepKind.rewound);
          expect(session.height, bottomHeight);
        }
        for (var top = 480; top >= 0; top -= 120) {
          final step = session.push(_frame(top));
          expect(step.kind, LongStepKind.appended);
          expect(step.direction, 'prepend-up');
        }
        expect(session.width, 160);
        expect(session.height, 2400);
        final directory = await Directory.systemTemp.createTemp(
          'snappin-rewind-',
        );
        try {
          final output = await session.savePng(
            File('${directory.path}/result.png'),
          );
          final image = img.decodePng(await output.readAsBytes())!;
          expect(image.width, 160);
          expect(image.height, 2400);
          for (final row in [0, 500, 1100, 1800, 2399]) {
            expect(
              image.getPixel(5, row).r.toInt(),
              img
                  .decodePng(_frame(row.clamp(0, 1800)))!
                  .getPixel(5, row - row.clamp(0, 1800))
                  .r
                  .toInt(),
            );
          }
        } finally {
          await directory.delete(recursive: true);
        }
      } finally {
        session.dispose();
      }
    },
  );

  test('memory limit retains full resolution stitched canvas', () async {
    final session = LongStitchSession.start(
      _frame(0),
      maxCanvasBytes: 160 * 840 * 4,
    );
    try {
      var reachedLimit = false;
      for (var top = 120; top <= 12000; top += 120) {
        try {
          session.push(_frame(top));
        } on LongCaptureLimitException {
          reachedLimit = true;
          break;
        }
      }
      expect(reachedLimit, isTrue);
      expect(session.width, 160);
      expect(session.height, 840);
      final directory = await Directory.systemTemp.createTemp(
        'snappin-long-limit-',
      );
      try {
        final output = await session.savePng(
          File('${directory.path}/result.png'),
        );
        expect(
          img.decodePng(await output.readAsBytes())?.height,
          session.height,
        );
      } finally {
        await directory.delete(recursive: true);
      }
    } finally {
      session.dispose();
    }
  });

  test('scrollbar jump never appends a coincidental tail overlap', () {
    final session = LongStitchSession.start(_frame(600));
    try {
      for (var top = 720; top <= 1800; top += 120) {
        expect(session.push(_frame(top)).kind, LongStepKind.appended);
      }
      final height = session.height;
      expect(session.push(_frame(0)).kind, LongStepKind.noMatch);
      for (final top in [120, 240, 360, 480]) {
        final step = session.push(_frame(top));
        expect(step.kind, LongStepKind.noMatch, reason: 'top=$top');
        expect(session.height, height);
      }
      final anchored = session.push(_frame(601));
      expect(anchored.kind, LongStepKind.appended);
      expect(anchored.direction, 'recovered-prepend');
      expect(session.height, height + 600);
      expect(session.push(_frame(721)).kind, LongStepKind.rewound);
      expect(session.height, height + 600);
    } finally {
      session.dispose();
    }
  });

  test(
    'jump below canvas recovers the skipped tail without moving the anchor',
    () async {
      final session = LongStitchSession.start(_frame(600));
      final directory = await Directory.systemTemp.createTemp(
        'snappin-reconnect-',
      );
      try {
        for (var top = 720; top <= 1800; top += 120) {
          session.push(_frame(top));
        }
        expect(session.push(_frame(3000)).kind, LongStepKind.noMatch);
        for (var top = 2880; top > 1800; top -= 120) {
          expect(session.push(_frame(top)).kind, LongStepKind.noMatch);
        }
        final recovery = session.push(_frame(1800));
        expect(recovery.direction, 'recovered-append');
        expect(session.height, 3000);
        // Continue beyond the original top after reconnecting below the canvas.
        for (var top = 1680; top >= 0; top -= 120) {
          session.push(_frame(top));
        }
        expect(session.height, 3600);
        final file = await session.savePng(
          File('${directory.path}/result.png'),
        );
        final output = img.decodePng(await file.readAsBytes())!;
        for (final top in [0, 600, 1800, 2400, 3000]) {
          final original = img.decodePng(_frame(top))!;
          for (final row in [0, 100, 599]) {
            final actual = output.getPixel(25, top + row);
            final expected = original.getPixel(25, row);
            expect(
              [actual.r, actual.g, actual.b],
              [expected.r, expected.g, expected.b],
              reason: 'global row=${top + row}',
            );
          }
        }
      } finally {
        session.dispose();
        await directory.delete(recursive: true);
      }
    },
  );

  test('background stitch worker exports original pixels and remains usable after bad input', () async {
    final directory = await Directory.systemTemp.createTemp('snappin-worker-');
    final worker = await LongStitchWorker.start(_frame(0));
    try {
      expect(worker.width, 160);
      expect(worker.height, 600);
      expect((await worker.push(_frame(120))).kind, LongStepKind.appended);
      expect(worker.height, 720);
      await expectLater(worker.push(Uint8List(0)), throwsStateError);
      expect((await worker.push(_frame(240))).kind, LongStepKind.appended);
      expect(worker.height, 840);
      final firstTile = img.decodePng(await worker.firstTilePng())!;
      expect((firstTile.width, firstTile.height), (160, 840));
      expect(
        firstTile.getPixel(30, 0).r,
        img.decodePng(_frame(0))!.getPixel(30, 0).r,
      );
      final preview = img.decodePng(await worker.previewPng())!;
      expect(preview.height, lessThan(worker.height));
      final file = await worker.savePng(File('${directory.path}/worker.png'));
      final result = img.decodePng(await file.readAsBytes())!;
      expect(firstTile.getBytes(), result.getBytes());
      expect(result.width, 160);
      expect(result.height, 840);
      expect(
        result.getPixel(30, 839).r,
        img.decodePng(_frame(240))!.getPixel(30, 599).r,
      );
    } finally {
      worker.dispose();
      await directory.delete(recursive: true);
    }
    await expectLater(worker.previewPng(), throwsStateError);
  });

  test(
    'background memory limit preserves the last full resolution image',
    () async {
      final worker = await LongStitchWorker.start(
        _frame(0),
        maxCanvasBytes: 160 * 720 * 4,
      );
      final directory = await Directory.systemTemp.createTemp(
        'snappin-worker-limit-',
      );
      try {
        await worker.push(_frame(120));
        await expectLater(
          worker.push(_frame(240)),
          throwsA(isA<LongCaptureLimitException>()),
        );
        final file = await worker.savePng(
          File('${directory.path}/retained.png'),
        );
        expect(img.decodePng(await file.readAsBytes())!.height, 720);
      } finally {
        worker.dispose();
        await directory.delete(recursive: true);
      }
    },
  );
}
