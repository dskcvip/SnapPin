import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

/// PNG output with one RGBA scanline of working memory. Compression and file
/// writing happen in the stitch isolate; no full-sized Image or PNG is copied.
Future<void> writePngRows(
  File target,
  int width,
  int height,
  Uint8List Function(int) row,
) async {
  await target.parent.create(recursive: true);
  final staging = await target.parent.createTemp('.snappin-png-');
  final file = File('${staging.path}/image.png');
  try {
    final output = await file.open(mode: FileMode.write);
    try {
      output.writeFromSync([137, 80, 78, 71, 13, 10, 26, 10]);
      final header = ByteData(13)
        ..setUint32(0, width)
        ..setUint32(4, height)
        ..setUint8(8, 8)
        ..setUint8(9, 6);
      _chunk(output, 'IHDR', header.buffer.asUint8List());
      final compressor = ZLibEncoder(level: 1)
          .startChunkedConversion(_PngData(output));
      final scanline = Uint8List(width * 4 + 1); // filter type 0, lossless
      for (var y = 0; y < height; y++) {
        scanline.setRange(1, scanline.length, row(y));
        compressor.add(scanline);
      }
      compressor.close();
      _chunk(output, 'IEND', const []);
      await output.flush();
    } finally {
      await output.close();
    }
    await file.rename(target.path);
  } finally {
    if (await file.exists()) await file.delete();
    await staging.delete();
  }
}

class _PngData implements Sink<List<int>> {
  _PngData(this.file);
  final RandomAccessFile file;
  @override
  void add(List<int> data) {
    for (var offset = 0; offset < data.length; offset += 65536) {
      final end = offset + 65536 < data.length ? offset + 65536 : data.length;
      _chunk(file, 'IDAT', data.sublist(offset, end));
    }
  }

  @override
  void close() {}
}

final _crcTable = List<int>.generate(256, (n) {
  for (var i = 0; i < 8; i++) {
    n = (n & 1) != 0 ? 0xedb88320 ^ (n >>> 1) : n >>> 1;
  }
  return n;
});

void _chunk(RandomAccessFile file, String type, List<int> data) {
  final name = ascii.encode(type);
  var crc = 0xffffffff;
  for (final byte in name) {
    crc = _crcTable[(crc ^ byte) & 255] ^ (crc >>> 8);
  }
  for (final byte in data) {
    crc = _crcTable[(crc ^ byte) & 255] ^ (crc >>> 8);
  }
  file.writeFromSync(
    (ByteData(4)..setUint32(0, data.length)).buffer.asUint8List(),
  );
  file.writeFromSync(name);
  file.writeFromSync(data);
  file.writeFromSync(
    (ByteData(4)..setUint32(0, crc ^ 0xffffffff)).buffer.asUint8List(),
  );
}
