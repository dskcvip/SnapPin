import 'dart:ui' as ui;
import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/windows/mosaic_preview.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test('mosaic preview paints averaged pixels while drawing', () async {
    final pixels = Uint8List.fromList([
      for (var y = 0; y < 8; y++)
        for (var x = 0; x < 8; x++) ...[x.isOdd ? 255 : 0, 0, 0, 255],
    ]);
    final preview = MosaicPreview(8, 8, pixels);
    final recorder = ui.PictureRecorder();
    preview.paint(
      ui.Canvas(recorder),
      const ui.Rect.fromLTWH(0, 0, 8, 8),
      const ui.Size(8, 8),
      4,
    );
    final image = await recorder.endRecording().toImage(8, 8);
    final bytes = (await image.toByteData(format: ui.ImageByteFormat.rawRgba))!;
    expect(bytes.getUint8((1 * 8 + 1) * 4), 127);
    expect(bytes.getUint8((2 * 8 + 2) * 4), 127);
    expect(bytes.getUint8((7 * 8 + 7) * 4 + 3), 255);
    image.dispose();
  });
}
