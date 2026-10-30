import 'dart:convert';
import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

class PinManagerWindow extends StatefulWidget {
  const PinManagerWindow({super.key, required this.controller});

  final WindowController controller;

  @override
  State<PinManagerWindow> createState() => _PinManagerWindowState();
}

class _PinManagerWindowState extends State<PinManagerWindow> {
  List<_PinEntry>? _pins;
  String? _busy;

  @override
  void initState() {
    super.initState();
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'focusManager') {
        await windowManager.show();
        await windowManager.focus();
        _reload();
      }
      return null;
    });
    _reload();
  }

  Future<void> _reload() async {
    final entries = <_PinEntry>[];
    for (final window in await WindowController.getAll()) {
      if (window.arguments.isEmpty) continue;
      try {
        if ((jsonDecode(window.arguments) as Map)['role'] != 'pin') continue;
        final result = await window.invokeMethod<String>('getPinInfo');
        if (result != null) {
          entries.add(_PinEntry(window, jsonDecode(result) as Map));
        }
      } catch (_) {
        // A pin can close while the list is refreshing.
      }
    }
    if (mounted) setState(() => _pins = entries);
  }

  Future<void> _run(_PinEntry pin, String action, [String? value]) async {
    setState(() => _busy = pin.id);
    try {
      await pin.window.invokeMethod<void>(action, value);
      await _reload();
    } finally {
      if (mounted) setState(() => _busy = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final pins = _pins;
    return Container(
      color: Colors.white,
      padding: const EdgeInsets.fromLTRB(12, 10, 12, 12),
      child: Column(
        children: [
          Row(
            children: [
              const Text(
                '贴图管理',
                style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
              ),
              const SizedBox(width: 8),
              Text(
                pins == null ? '读取中…' : '${pins.length} 张',
                style: const TextStyle(fontSize: 12, color: Color(0xff8b929c)),
              ),
              const Spacer(),
              _actionButton('刷新', _reload),
            ],
          ),
          const Divider(height: 10, color: Color(0xffe9ecf0)),
          Expanded(
            child: pins == null
                ? const Center(child: CircularProgressIndicator())
                : pins.isEmpty
                ? const Padding(
                    padding: EdgeInsets.only(top: 40),
                    child: Text(
                      '还没有贴图。\n截图后点工具条的「贴图」，或按 Ctrl+2 把剪贴板图片贴到屏幕上。',
                      textAlign: TextAlign.center,
                      style: TextStyle(
                        fontSize: 12,
                        height: 1.8,
                        color: Color(0x730f172a),
                      ),
                    ),
                  )
                : ListView.separated(
                    padding: const EdgeInsets.only(top: 8),
                    itemCount: pins.length,
                    separatorBuilder: (_, _) => const SizedBox(height: 8),
                    itemBuilder: (_, index) => _pinRow(pins[index]),
                  ),
          ),
        ],
      ),
    );
  }

  Widget _pinRow(_PinEntry pin) => Container(
    height: 66,
    padding: const EdgeInsets.all(8),
    decoration: BoxDecoration(
      color: const Color(0xfffafafa),
      border: Border.all(color: const Color(0xffe9ecf0)),
      borderRadius: BorderRadius.circular(9),
    ),
    child: Row(
      children: [
        InkWell(
          onTap: () => _run(pin, 'focusPin'),
          child: Container(
            width: 64,
            height: 48,
            decoration: BoxDecoration(
              color: Colors.white,
              border: Border.all(color: const Color(0xffe1e4e9)),
              borderRadius: BorderRadius.circular(6),
            ),
            child: Image.file(
              File(pin.path),
              fit: BoxFit.contain,
              errorBuilder: (_, _, _) => const SizedBox.shrink(),
            ),
          ),
        ),
        const SizedBox(width: 10),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                '${pin.width.round()} × ${pin.height.round()}',
                style: const TextStyle(fontSize: 12),
              ),
              SizedBox(
                height: 27,
                child: TextFormField(
                  key: ValueKey(pin.id),
                  initialValue: pin.group,
                  style: const TextStyle(fontSize: 12),
                  decoration: const InputDecoration(
                    isDense: true,
                    contentPadding: EdgeInsets.symmetric(
                      horizontal: 7,
                      vertical: 5,
                    ),
                    border: OutlineInputBorder(),
                  ),
                  onFieldSubmitted: (value) =>
                      _run(pin, 'setPinGroup', value.trim()),
                  onChanged: (value) => pin.window.invokeMethod<void>(
                    'setPinGroup',
                    value.trim(),
                  ),
                  onTapOutside: (_) => _reload(),
                ),
              ),
            ],
          ),
        ),
        const SizedBox(width: 8),
        _actionButton(
          '置前',
          _busy == pin.id ? null : () => _run(pin, 'focusPin'),
        ),
        const SizedBox(width: 6),
        _actionButton(
          '关闭',
          _busy == pin.id ? null : () => _run(pin, 'closePin'),
          danger: true,
        ),
      ],
    ),
  );

  Widget _actionButton(
    String label,
    VoidCallback? action, {
    bool danger = false,
  }) => OutlinedButton(
    onPressed: action,
    style: ButtonStyle(
      foregroundColor: WidgetStateProperty.resolveWith((states) {
        if (states.contains(WidgetState.hovered)) {
          return danger ? Colors.white : const Color(0xff12a5d8);
        }
        return const Color(0xb80f172a);
      }),
      backgroundColor: WidgetStateProperty.resolveWith(
        (states) => danger && states.contains(WidgetState.hovered)
            ? const Color(0xfffa5151)
            : Colors.white,
      ),
      side: WidgetStateProperty.resolveWith(
        (states) => BorderSide(
          color: states.contains(WidgetState.hovered)
              ? danger
                    ? const Color(0xfffa5151)
                    : const Color(0xff12a5d8)
              : const Color(0x240f172a),
        ),
      ),
      minimumSize: WidgetStateProperty.all(const Size(0, 28)),
      padding: WidgetStateProperty.all(
        const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
      ),
      shape: WidgetStateProperty.all(
        RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
      ),
    ),
    child: Text(label, style: const TextStyle(fontSize: 12)),
  );
}

class _PinEntry {
  _PinEntry(this.window, Map data)
    : id = data['id'].toString(),
      path = data['path'] as String,
      width = (data['width'] as num).toDouble(),
      height = (data['height'] as num).toDouble(),
      group = data['group'] as String;

  final WindowController window;
  final String id;
  final String path;
  final double width;
  final double height;
  final String group;
}
