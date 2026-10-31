import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import '../window_roles.dart';
import '../ocr_image_store.dart';
import '../app_log.dart';
import '../ocr_document.dart';

Future<void> showOcrResult(
  Uint8List png,
  String text, {
  OcrDocument? document,
}) async {
  final image = await OcrImageStore.create(png);
  try {
    final payload = {
      'image': image.path,
      'text': text,
      if (document != null) 'document': document.toJson(),
    };
    for (final window in await WindowController.getAll()) {
      if (windowRole(window) == 'ocr') {
        await window.invokeMethod<void>('openResult', payload);
        return;
      }
    }
    await WindowController.create(
      WindowConfiguration(
        arguments: jsonEncode({'role': 'ocr', ...payload}),
        hiddenAtLaunch: true,
      ),
    );
  } catch (_) {
    await OcrImageStore.deleteOwned(image.path);
    rethrow;
  }
}

class OcrWindow extends StatefulWidget {
  const OcrWindow({
    super.key,
    required this.controller,
    required this.imagePath,
    required this.text,
    this.document,
  });

  final WindowController controller;
  final String imagePath;
  final String text;
  final OcrDocument? document;

  @override
  State<OcrWindow> createState() => _OcrWindowState();
}

class _OcrWindowState extends State<OcrWindow> {
  bool _copied = false;
  bool _topmost = true;
  late String _imagePath = widget.imagePath;
  late String _text = widget.text;
  late OcrDocument? _document = widget.document;
  bool _keepLayout = true;

  @override
  void initState() {
    super.initState();
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'openResult') {
        final data = Map<String, dynamic>.from(call.arguments as Map);
        final oldPath = _imagePath;
        setState(() {
          _imagePath = data['image'] as String;
          _text = data['text'] as String;
          _document = data['document'] is Map
              ? OcrDocument.fromJson(data['document'] as Map)
              : null;
          _keepLayout = true;
          _copied = false;
        });
        if (oldPath != _imagePath) unawaited(_deleteImage(oldPath));
        await windowManager.setAlwaysOnTop(true);
        if (mounted) setState(() => _topmost = true);
        await windowManager.show();
        await windowManager.focus();
      }
      return null;
    });
  }

  @override
  void dispose() {
    unawaited(_deleteImage(_imagePath));
    super.dispose();
  }

  Future<void> _deleteImage(String path) async {
    await OcrImageStore.deleteOwned(path);
  }

  Future<void> _copyAll() async {
    await Clipboard.setData(ClipboardData(text: _text));
    if (!mounted) return;
    setState(() => _copied = true);
    unawaited(
      Future<void>.delayed(const Duration(milliseconds: 1500), () {
        if (mounted) setState(() => _copied = false);
      }),
    );
  }

  Future<void> _toggleTopmost() async {
    final next = !_topmost;
    await windowManager.setAlwaysOnTop(next);
    if (mounted) setState(() => _topmost = next);
    AppLog.write('ocr-topmost', next);
  }

  Future<void> _translate() async {
    for (final window in await WindowController.getAll()) {
      if (windowRole(window) == 'translation') {
        await window.invokeMethod<void>('openSource', _text);
        return;
      }
    }
    await WindowController.create(
      WindowConfiguration(
        arguments: jsonEncode({'role': 'translation', 'source': _text}),
        hiddenAtLaunch: true,
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final count = _text.trim().isEmpty
        ? 0
        : _text.trimRight().split('\n').length;
    return ColoredBox(
      color: Colors.white,
      child: Column(
        children: [
          GestureDetector(
            behavior: HitTestBehavior.opaque,
            onPanStart: (_) => windowManager.startDragging(),
            child: Container(
              height: 53,
              padding: const EdgeInsets.symmetric(horizontal: 16),
              decoration: const BoxDecoration(
                border: Border(bottom: BorderSide(color: Color(0xffeceef1))),
              ),
              child: Row(
                children: [
                  const Text(
                    '文字识别',
                    style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
                  ),
                  if (count > 0) ...[
                    const SizedBox(width: 8),
                    Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 7,
                        vertical: 1,
                      ),
                      decoration: BoxDecoration(
                        color: const Color(0xfff0f1f3),
                        borderRadius: BorderRadius.circular(20),
                      ),
                      child: Text(
                        '$count 行',
                        style: const TextStyle(
                          fontSize: 11,
                          color: Color(0xff6b7280),
                        ),
                      ),
                    ),
                  ],
                  const Spacer(),
                  if (_document?.hasLayout == true)
                    TextButton(
                      onPressed: () =>
                          setState(() => _keepLayout = !_keepLayout),
                      child: Text(
                        _keepLayout ? '保留排版' : '纯文本',
                        style: const TextStyle(fontSize: 12),
                      ),
                    ),
                  IconButton(
                    tooltip: _topmost ? '取消置顶' : '窗口置顶',
                    onPressed: _toggleTopmost,
                    iconSize: 17,
                    color: _topmost
                        ? const Color(0xff12a5d8)
                        : const Color(0xff6b7280),
                    icon: Icon(
                      _topmost ? Icons.push_pin : Icons.push_pin_outlined,
                    ),
                  ),
                  TextButton(
                    onPressed: windowManager.close,
                    style: TextButton.styleFrom(
                      foregroundColor: const Color(0xff6b7280),
                      backgroundColor: Colors.transparent,
                      minimumSize: const Size(0, 28),
                      padding: const EdgeInsets.symmetric(horizontal: 11),
                      shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(7),
                      ),
                    ),
                    child: const Text('关闭', style: TextStyle(fontSize: 12)),
                  ),
                ],
              ),
            ),
          ),
          Expanded(
            child: Row(
              children: [
                Expanded(
                  flex: 42,
                  child: Container(
                    width: double.infinity,
                    height: double.infinity,
                    alignment: Alignment.center,
                    padding: const EdgeInsets.all(12),
                    decoration: const BoxDecoration(
                      color: Color(0xfff4f5f7),
                      border: Border(
                        right: BorderSide(color: Color(0xffeceef1)),
                      ),
                    ),
                    child: Container(
                      decoration: BoxDecoration(
                        color: Colors.white,
                        borderRadius: BorderRadius.circular(4),
                        boxShadow: const [
                          BoxShadow(
                            color: Color(0x1f0f172a),
                            blurRadius: 4,
                            offset: Offset(0, 1),
                          ),
                        ],
                      ),
                      child: Image.file(
                        File(_imagePath),
                        fit: BoxFit.contain,
                        errorBuilder: (_, _, _) => const Text(
                          '原图文件已被清理，重新识别即可',
                          style: TextStyle(
                            color: Color(0xff9aa0ab),
                            fontSize: 12,
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
                Expanded(
                  flex: 58,
                  child: _keepLayout && _document?.hasLayout == true
                      ? OcrLayoutView(document: _document!)
                      : SingleChildScrollView(
                          padding: const EdgeInsets.symmetric(
                            horizontal: 18,
                            vertical: 14,
                          ),
                          child: SizedBox(
                            width: double.infinity,
                            child: SelectableText(
                              _text.isEmpty ? '没有识别结果。重新截图识别后再来。' : _text,
                              style: const TextStyle(
                                fontSize: 13,
                                height: 1.75,
                                color: Color(0xff1f2329),
                              ),
                            ),
                          ),
                        ),
                ),
              ],
            ),
          ),
          Container(
            height: 59,
            padding: const EdgeInsets.symmetric(horizontal: 12),
            decoration: const BoxDecoration(
              color: Color(0xfffafbfc),
              border: Border(top: BorderSide(color: Color(0xffeceef1))),
            ),
            child: Row(
              children: [
                const Expanded(
                  child: Text(
                    '拖动鼠标选中任意内容，Ctrl+C 复制',
                    style: TextStyle(fontSize: 11, color: Color(0xff9aa0ab)),
                  ),
                ),
                _action(
                  Icons.copy,
                  _copied ? '已复制' : '复制全文',
                  _text.isEmpty ? null : _copyAll,
                ),
                _action(
                  Icons.translate,
                  '翻译',
                  _text.isEmpty ? null : _translate,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Widget _action(IconData icon, String label, VoidCallback? action) => SizedBox(
    width: 64,
    child: TextButton(
      onPressed: action,
      style: TextButton.styleFrom(
        foregroundColor: const Color(0xff4e5566),
        padding: const EdgeInsets.symmetric(vertical: 5),
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: 16),
          const SizedBox(height: 3),
          Text(label, style: const TextStyle(fontSize: 11)),
        ],
      ),
    ),
  );
}
