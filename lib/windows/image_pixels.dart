import 'dart:io';
import 'dart:isolate';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:image/image.dart' as img;

import '../app_log.dart';

class ImagePixels extends StatefulWidget {
  const ImagePixels({
    super.key,
    required this.path,
    required this.sourceHeight,
    required this.displayWidth,
    required this.scale,
    this.opacity = 1,
    this.firstTilePath,
  });

  final String path;
  final int sourceHeight;
  final double displayWidth;
  final double scale;
  final double opacity;
  final String? firstTilePath;

  @override
  State<ImagePixels> createState() => _ImagePixelsState();
}

class _ImagePixelsState extends State<ImagePixels> {
  Future<List<Uint8List>>? _tiles;

  @override
  void didUpdateWidget(covariant ImagePixels oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.path != widget.path) _tiles = null;
  }

  Future<List<Uint8List>> _loadTiles() async {
    final watch = Stopwatch()..start();
    final bytes = await File(widget.path).readAsBytes();
    final result = await Isolate.run(() {
      final source = img.decodePng(bytes);
      if (source == null) throw StateError('图片无法解码');
      final tiles = <Uint8List>[];
      for (var y = 0; y < source.height; y += 1024) {
        final tile = img.copyCrop(
          source,
          x: 0,
          y: y,
          width: source.width,
          height: math.min(1024, source.height - y),
        );
        tiles.add(Uint8List.fromList(img.encodePng(tile, level: 1)));
      }
      return tiles;
    });
    AppLog.write(
      'viewer-tiles-ready',
      'tiles=${result.length} ${watch.elapsedMilliseconds}ms',
    );
    return result;
  }

  @override
  Widget build(BuildContext context) => Opacity(
    opacity: widget.opacity,
    child: widget.sourceHeight <= 8192
        ? Image.file(
            File(widget.path),
            key: ValueKey(widget.path),
            fit: BoxFit.fill,
            filterQuality: FilterQuality.medium,
          )
        : FutureBuilder<List<Uint8List>>(
            future: _tiles ??= _loadTiles(),
            builder: (context, snapshot) {
              if (snapshot.hasError) {
                return Center(child: Text('图片加载失败：${snapshot.error}'));
              }
              if (!snapshot.hasData) {
                return Align(
                  alignment: Alignment.topLeft,
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      if (widget.firstTilePath != null)
                        Image.file(
                          File(widget.firstTilePath!),
                          width: widget.displayWidth,
                          height:
                              math.min(1024, widget.sourceHeight) *
                              widget.scale,
                          fit: BoxFit.fill,
                          gaplessPlayback: true,
                        ),
                      const Padding(
                        padding: EdgeInsets.all(16),
                        child: Text(
                          '正在加载完整长图…',
                          style: TextStyle(
                            fontSize: 12,
                            color: Color(0xff6b7280),
                          ),
                        ),
                      ),
                    ],
                  ),
                );
              }
              return Column(
                children: [
                  for (var i = 0; i < snapshot.data!.length; i++)
                    Image.memory(
                      snapshot.data![i],
                      width: widget.displayWidth,
                      height:
                          math.min(1024, widget.sourceHeight - i * 1024) *
                          widget.scale,
                      fit: BoxFit.fill,
                      filterQuality: FilterQuality.medium,
                      gaplessPlayback: true,
                    ),
                ],
              );
            },
          ),
  );
}
