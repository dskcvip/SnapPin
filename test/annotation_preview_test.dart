import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/annotation.dart';
import 'package:snappin_flutter/annotation_preview.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  late ui.Image source;
  late AnnotationPreview preview;
  setUp(() async {
    final ready = ui.PictureRecorder();
    final canvas = Canvas(ready);
    for (var x = 0; x < 64; x++) {
      canvas.drawRect(
        Rect.fromLTWH(x.toDouble(), 0, 1, 64),
        Paint()..color = Color.fromARGB(255, x * 4, x * 4, x * 4),
      );
    }
    final picture = ready.endRecording();
    source = await picture.toImage(64, 64);
    picture.dispose();
    preview = AnnotationPreview(
      source,
      (await source.toByteData(format: ui.ImageByteFormat.rawRgba))!,
      const Size(64, 64),
    );
  });
  tearDown(() {
    preview.dispose();
    source.dispose();
  });

  Future<Uint8List> render(List<Annotation> marks, [Annotation? draft]) async {
    final recorder = ui.PictureRecorder();
    preview.paint(Canvas(recorder), marks, draft);
    final picture = recorder.endRecording();
    final image = await picture.toImage(64, 64);
    picture.dispose();
    final bytes = (await image.toByteData(format: ui.ImageByteFormat.rawRgba))!
        .buffer
        .asUint8List();
    image.dispose();
    return bytes;
  }

  Annotation effect(AnnotationTool tool, double parameter) => Annotation(
    tool: tool,
    points: const [Offset(16, 16), Offset(48, 48)],
    color: const Color(0xffff0000),
    strokeWidth: 2,
    parameter: parameter,
  );

  test(
    'mosaic changes pixels during dragging and matches committed preview',
    () async {
      final mark = effect(AnnotationTool.mosaic, 12);
      final dragging = await render([], mark);
      expect(dragging[(22 * 64 + 18) * 4], 86);
      expect(await render([mark]), dragging);
      final undone = await render([]);
      expect(undone[(22 * 64 + 18) * 4], 72);
      expect(await render([mark]), dragging);
    },
  );
  test(
    'spotlight preserves hole, dims outside and rounds the corners',
    () async {
      final data = await render([], effect(AnnotationTool.spotlight, 150));
      expect(data[(32 * 64 + 32) * 4], 128);
      expect(data[(2 * 64 + 32) * 4], closeTo(53, 3));
      expect(data[(17 * 64 + 17) * 4], lessThan(45));
    },
  );
  test(
    'eraser restores complete sparse stroke before finishing screenshot',
    () async {
      const line = Annotation(
        tool: AnnotationTool.pen,
        points: [Offset(10, 32), Offset(54, 32)],
        color: Color(0xffff0000),
        strokeWidth: 6,
      );
      const eraser = Annotation(
        tool: AnnotationTool.eraser,
        points: [Offset(16, 32), Offset(48, 32)],
        color: Color(0xffff0000),
        strokeWidth: 2,
        parameter: 4,
      );
      final data = await render([line], eraser);
      for (var x = 16; x <= 48; x++) {
        expect(data[(32 * 64 + x) * 4], x * 4, reason: 'Gap at x=$x');
        expect(data[(32 * 64 + x) * 4 + 1], x * 4);
      }
      expect(data[(32 * 64 + 10) * 4], 255);
      expect(data[(32 * 64 + 10) * 4 + 1], 0);
    },
  );
  test('smart erase uses original boundary gradient during dragging', () async {
    const damage = Annotation(
      tool: AnnotationTool.rectangle,
      points: [Offset(16, 16), Offset(48, 48)],
      color: Color(0xffff0000),
      strokeWidth: 6,
    );
    final data = await render([damage], effect(AnnotationTool.smartErase, 0));
    expect(data[(32 * 64 + 32) * 4], closeTo(128, 5));
    expect(data[(32 * 64 + 32) * 4], data[(32 * 64 + 32) * 4 + 1]);
  });
}
