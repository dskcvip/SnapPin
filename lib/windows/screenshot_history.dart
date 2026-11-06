import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../app_log.dart';
import '../window_roles.dart';

/// History decodes only visible thumbnails, never changes archived originals.
class ScreenshotHistory extends StatefulWidget {
  const ScreenshotHistory({
    super.key,
    required this.currentPath,
    this.revision = 0,
    required this.onSelected,
  });
  final String currentPath;
  final int revision;
  final Future<void> Function(Map<String, dynamic>) onSelected;

  @override
  State<ScreenshotHistory> createState() => _ScreenshotHistoryState();
}

class _ScreenshotHistoryState extends State<ScreenshotHistory> {
  late Future<List<Map<String, dynamic>>> _entries = _load();
  bool _newestFirst = true;
  Timer? _hoverTimer;
  OverlayEntry? _preview;

  void _hidePreview() {
    _hoverTimer?.cancel();
    _preview?.remove();
    _preview?.dispose();
    _preview = null;
  }

  void _showPreview(BuildContext anchor, Map<String, dynamic> entry) {
    _hidePreview();
    _hoverTimer = Timer(const Duration(milliseconds: 220), () {
      if (!mounted || !anchor.mounted) return;
      final box = anchor.findRenderObject() as RenderBox;
      final origin = box.localToGlobal(Offset.zero);
      final size = MediaQuery.sizeOf(context);
      final width = (origin.dx - 20).clamp(100.0, 360.0);
      final height = (size.height - 24).clamp(100.0, 420.0);
      final top = origin.dy.clamp(12.0, size.height - height - 12);
      _preview = OverlayEntry(
        builder: (_) => Positioned(
          left: (origin.dx - width - 12).clamp(4.0, size.width - width - 4),
          top: top,
          width: width,
          height: height,
          child: IgnorePointer(
            child: Material(
              key: const ValueKey('screenshot-hover-preview'),
              color: Colors.white,
              elevation: 5,
              shadowColor: const Color(0x33000000),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(5),
                side: const BorderSide(color: Color(0xffd8d8d8)),
              ),
              child: Padding(
                padding: const EdgeInsets.all(8),
                child: Column(
                  children: [
                    Expanded(
                      child: Image.file(
                        File(entry['path'] as String),
                        fit: BoxFit.contain,
                        cacheWidth: _decodeSize(entry, 720, 840).width.round(),
                        cacheHeight: _decodeSize(
                          entry,
                          720,
                          840,
                        ).height.round(),
                        errorBuilder: (_, _, _) => const Icon(
                          Icons.broken_image_outlined,
                          color: Colors.grey,
                        ),
                      ),
                    ),
                    const SizedBox(height: 6),
                    Text(
                      '${entry['width']} × ${entry['height']} px',
                      style: const TextStyle(
                        fontSize: 11,
                        color: Color(0xff777777),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      );
      Overlay.of(context).insert(_preview!);
    });
  }

  @override
  void dispose() {
    _hidePreview();
    super.dispose();
  }

  @override
  void didUpdateWidget(covariant ScreenshotHistory oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.revision != widget.revision) {
      _hidePreview();
      _entries = _load();
    }
  }

  Future<List<Map<String, dynamic>>> _load() async {
    final ball = await findBallWindow();
    if (ball == null) throw StateError('主窗口不可用');
    final result = await ball.invokeMethod<String>('listScreenshots');
    final entries = (jsonDecode(result ?? '[]') as List)
        .map((item) => Map<String, dynamic>.from(item as Map))
        .toList();
    AppLog.write('pin-history-loaded', 'count=${entries.length}');
    return entries;
  }

  @override
  Widget build(BuildContext context) => Container(
    key: const ValueKey('screenshot-history'),
    width: 216,
    decoration: const BoxDecoration(
      color: Color(0xfff7f7f7),
      border: Border(left: BorderSide(color: Color(0xffdedede))),
    ),
    child: Column(
      children: [
        SizedBox(
          height: 34,
          child: Row(
            children: [
              const SizedBox(width: 10),
              const Text(
                '截图历史',
                style: TextStyle(fontSize: 12, color: Color(0xff666666)),
              ),
              const Spacer(),
              IconButton(
                tooltip: _newestFirst ? '日期：从新到旧' : '日期：从旧到新',
                padding: EdgeInsets.zero,
                constraints: const BoxConstraints.tightFor(
                  width: 30,
                  height: 30,
                ),
                iconSize: 16,
                onPressed: () => setState(() => _newestFirst = !_newestFirst),
                icon: Icon(
                  _newestFirst ? Icons.arrow_downward : Icons.arrow_upward,
                ),
              ),
              IconButton(
                tooltip: '刷新截图历史',
                padding: EdgeInsets.zero,
                constraints: const BoxConstraints.tightFor(
                  width: 30,
                  height: 30,
                ),
                iconSize: 16,
                onPressed: () => setState(() => _entries = _load()),
                icon: const Icon(Icons.refresh),
              ),
            ],
          ),
        ),
        Expanded(
          child: FutureBuilder<List<Map<String, dynamic>>>(
            future: _entries,
            builder: (context, snapshot) {
              if (snapshot.hasError)
                return Center(
                  child: TextButton(
                    onPressed: () => setState(() => _entries = _load()),
                    child: const Text('加载失败，点击重试'),
                  ),
                );
              if (!snapshot.hasData)
                return const Center(
                  child: SizedBox(
                    width: 18,
                    height: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                );
              if (snapshot.data!.isEmpty)
                return const Center(
                  child: Text(
                    '暂无归档截图',
                    style: TextStyle(fontSize: 12, color: Colors.grey),
                  ),
                );
              final entries = _newestFirst
                  ? snapshot.data!
                  : snapshot.data!.reversed.toList();
              final rows = <Widget>[];
              var index = 0;
              while (index < entries.length) {
                final day = entries[index]['day'] as String;
                rows.add(
                  Padding(
                    padding: const EdgeInsets.fromLTRB(0, 8, 0, 6),
                    child: Text(
                      day,
                      style: const TextStyle(
                        fontSize: 11,
                        color: Color(0xff777777),
                      ),
                    ),
                  ),
                );
                final group = <Map<String, dynamic>>[];
                while (index < entries.length && entries[index]['day'] == day) {
                  group.add(entries[index++]);
                }
                for (var start = 0; start < group.length; start += 3) {
                  rows.add(
                    Padding(
                      padding: const EdgeInsets.only(bottom: 6),
                      child: Row(
                        children: [
                          for (var column = 0; column < 3; column++) ...[
                            if (column > 0) const SizedBox(width: 6),
                            Expanded(
                              child: start + column < group.length
                                  ? _thumbnail(group[start + column])
                                  : const SizedBox(height: 58),
                            ),
                          ],
                        ],
                      ),
                    ),
                  );
                }
              }
              return NotificationListener<ScrollStartNotification>(
                onNotification: (_) {
                  _hidePreview();
                  return false;
                },
                child: ListView.builder(
                  padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
                  itemCount: rows.length,
                  itemBuilder: (_, i) => rows[i],
                ),
              );
            },
          ),
        ),
      ],
    ),
  );

  Size _decodeSize(Map<String, dynamic> entry, int maxWidth, int maxHeight) {
    final width = (entry['width'] as num).toDouble();
    final height = (entry['height'] as num).toDouble();
    final scale = math.min(1.0, math.min(maxWidth / width, maxHeight / height));
    return Size(
      math.max(1, (width * scale).round()).toDouble(),
      math.max(1, (height * scale).round()).toDouble(),
    );
  }

  Widget _thumbnail(Map<String, dynamic> entry) => Builder(
    builder: (anchor) => MouseRegion(
      onEnter: (_) => _showPreview(anchor, entry),
      onExit: (_) => _hidePreview(),
      child: InkWell(
        onTap: () {
          _hidePreview();
          widget.onSelected(entry);
        },
        child: Container(
          key: ValueKey('archive-thumbnail-${entry['path']}'),
          height: 58,
          padding: const EdgeInsets.all(1),
          decoration: BoxDecoration(
            color: Colors.white,
            border: Border.all(
              color: entry['path'] == widget.currentPath
                  ? const Color(0xff27c36c)
                  : const Color(0xffeeeeee),
            ),
          ),
          child: ClipRect(
            child: Image.file(
              File(entry['path'] as String),
              fit: BoxFit.contain,
              cacheWidth: _decodeSize(entry, 120, 120).width.round(),
              cacheHeight: _decodeSize(entry, 120, 120).height.round(),
              filterQuality: FilterQuality.medium,
              errorBuilder: (_, _, _) => const Icon(
                Icons.broken_image_outlined,
                size: 18,
                color: Colors.grey,
              ),
            ),
          ),
        ),
      ),
    ),
  );
}
