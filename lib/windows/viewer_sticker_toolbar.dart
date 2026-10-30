import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_svg/flutter_svg.dart';

class ViewerStickerToolbar extends StatefulWidget {
  const ViewerStickerToolbar({super.key, required this.onSelected});
  final void Function(String glyph, String? svg, double size, Color color)
  onSelected;

  @override
  State<ViewerStickerToolbar> createState() => _ViewerStickerToolbarState();
}

class _ViewerStickerToolbarState extends State<ViewerStickerToolbar> {
  Map<String, String> _icons = {};
  Map<String, String> _zh = {};
  Map<String, List<String>> _synonyms = {};
  String _glyph = '✅';
  String? _svg;
  double _size = 48;
  Color _color = const Color(0xff111827);

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    final data = await Future.wait([
      rootBundle.loadString('assets/stickers.json'),
      rootBundle.loadString('assets/sticker_zh.json'),
    ]);
    if (!mounted) return;
    final translations = jsonDecode(data[1]) as Map;
    setState(() {
      _icons = Map<String, String>.from(jsonDecode(data[0]) as Map);
      _zh = Map<String, String>.from(translations['ZH'] as Map);
      _synonyms = (translations['CN_SYNONYMS'] as Map).map(
        (key, value) => MapEntry('$key', List<String>.from(value as List)),
      );
    });
  }

  String _label(String name) =>
      name.split('-').map((part) => _zh[part] ?? part).join();
  void _notify() => widget.onSelected(_glyph, _svg, _size, _color);

  @override
  Widget build(BuildContext context) => Container(
    height: 42,
    color: const Color(0xfff7f9f9),
    child: SingleChildScrollView(
      scrollDirection: Axis.horizontal,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      child: Row(
        children: [
          for (final glyph in const [
            '✅',
            '❌',
            '⚠️',
            '❓',
            '❗',
            '⭐',
            '❤️',
            '🔥',
            '👍',
            '👎',
            '💡',
            '📌',
            '🎯',
            '🚀',
            '😀',
            '😡',
          ])
            InkWell(
              onTap: () {
                setState(() {
                  _glyph = glyph;
                  _svg = null;
                });
                _notify();
              },
              child: Container(
                width: 28,
                height: 28,
                alignment: Alignment.center,
                decoration: BoxDecoration(
                  color: _svg == null && _glyph == glyph
                      ? const Color(0x1f12a5d8)
                      : null,
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Text(glyph, style: const TextStyle(fontSize: 16)),
              ),
            ),
          const SizedBox(width: 10),
          PopupMenuButton<Color>(
            tooltip: '贴纸颜色',
            onSelected: (color) {
              setState(() => _color = color);
              _notify();
            },
            itemBuilder: (_) => [
              for (final color in const [
                Colors.black,
                Colors.red,
                Colors.orange,
                Colors.green,
                Colors.blue,
                Colors.purple,
                Colors.white,
              ])
                PopupMenuItem(
                  value: color,
                  height: 30,
                  child: Icon(Icons.circle, color: color, size: 20),
                ),
            ],
            child: Icon(Icons.palette_outlined, color: _color, size: 22),
          ),
          const SizedBox(width: 10),
          PopupMenuButton<double>(
            tooltip: '贴纸尺寸',
            onSelected: (size) {
              setState(() => _size = size);
              _notify();
            },
            itemBuilder: (_) => [
              for (final size in [24.0, 36.0, 48.0, 64.0, 96.0, 128.0])
                PopupMenuItem(
                  value: size,
                  child: Text(
                    '${size.round()}px',
                    style: const TextStyle(fontSize: 12),
                  ),
                ),
            ],
            child: Text(
              '${_size.round()}px',
              style: const TextStyle(fontSize: 12),
            ),
          ),
          const SizedBox(width: 10),
          SizedBox(
            width: 190,
            height: 28,
            child: RawAutocomplete<String>(
              optionsBuilder: (query) {
                final term = query.text.trim().toLowerCase();
                return _icons.keys
                    .where(
                      (name) =>
                          term.isEmpty ||
                          name.contains(term) ||
                          _label(name).contains(term) ||
                          (_synonyms[term] ?? []).any(name.contains),
                    )
                    .take(60);
              },
              onSelected: (name) {
                setState(() => _svg = _icons[name]);
                _notify();
              },
              fieldViewBuilder: (_, controller, focus, submit) => TextField(
                controller: controller,
                focusNode: focus,
                style: const TextStyle(fontSize: 12),
                decoration: InputDecoration(
                  hintText: _icons.isEmpty
                      ? '加载图标…'
                      : '搜索 ${_icons.length} 个图标',
                  isDense: true,
                  filled: true,
                  fillColor: Colors.white,
                  contentPadding: const EdgeInsets.symmetric(
                    horizontal: 8,
                    vertical: 5,
                  ),
                  border: OutlineInputBorder(
                    borderRadius: BorderRadius.circular(6),
                  ),
                ),
              ),
              optionsViewBuilder: (_, select, options) => Align(
                alignment: Alignment.topLeft,
                child: Material(
                  elevation: 8,
                  color: const Color(0xfffafafa),
                  borderRadius: BorderRadius.circular(8),
                  child: SizedBox(
                    width: 264,
                    height: 204,
                    child: GridView.count(
                      crossAxisCount: 6,
                      padding: const EdgeInsets.all(6),
                      children: [
                        for (final name in options)
                          Tooltip(
                            message: '${_label(name)}  $name',
                            child: InkWell(
                              onTap: () => select(name),
                              child: Center(
                                child: SvgPicture.string(
                                  _icons[name]!,
                                  width: 18,
                                  height: 18,
                                  theme: SvgTheme(currentColor: _color),
                                ),
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    ),
  );
}
