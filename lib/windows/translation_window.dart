import 'dart:async';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import '../app_config.dart';
import '../window_roles.dart';

class TranslationWindow extends StatefulWidget {
  const TranslationWindow({super.key, this.controller, required this.source});
  final WindowController? controller;
  final String source;

  @override
  State<TranslationWindow> createState() => _TranslationWindowState();
}

class _TranslationWindowState extends State<TranslationWindow> {
  String? _translated;
  String? _error;
  String? _targetLang;
  String? _modelName;
  String? _copied;
  bool _busy = false;
  int _translationVersion = 0;
  late String _source = widget.source;
  late final TextEditingController _sourceController = TextEditingController(
    text: _source,
  );
  final TextEditingController _resultController = TextEditingController();
  final FocusNode _sourceFocus = FocusNode();
  final FocusNode _resultFocus = FocusNode();

  @override
  void initState() {
    super.initState();
    widget.controller?.setWindowMethodHandler((call) async {
      if (call.method == 'openSource') {
        final source = call.arguments as String;
        setState(() {
          _source = source;
          _sourceController.text = source;
          _resultController.clear();
          _translated = null;
          _error = null;
          _copied = null;
        });
        await windowManager.show();
        await windowManager.focus();
        await _refreshConfig();
        unawaited(_translate());
      }
      return null;
    });
    _refreshConfig();
    WidgetsBinding.instance.addPostFrameCallback((_) => _translate());
  }

  Future<void> _refreshConfig() async {
    final config = await ConfigStore.load();
    if (mounted) {
      setState(() {
        _targetLang = config.targetLang;
        _modelName = config.translationModels.firstOrNull?.name;
      });
    }
  }

  @override
  void dispose() {
    _sourceController.dispose();
    _resultController.dispose();
    _sourceFocus.dispose();
    _resultFocus.dispose();
    super.dispose();
  }

  Future<void> _translate() async {
    final version = ++_translationVersion;
    final source = _source;
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final ball = await findBallWindow();
      if (ball == null) throw StateError('主窗口不可用');
      final text = await ball.invokeMethod<String>('translateText', source);
      if (mounted && version == _translationVersion) {
        setState(() {
          _translated = text;
          _resultController.text = text ?? '';
        });
      }
    } catch (error) {
      if (mounted && version == _translationVersion) {
        setState(() => _error = error.toString());
      }
    } finally {
      if (mounted && version == _translationVersion) {
        setState(() => _busy = false);
      }
    }
  }

  Future<void> _copy(String which, String text) async {
    await Clipboard.setData(ClipboardData(text: text));
    if (!mounted) return;
    setState(() => _copied = which);
    Future<void>.delayed(const Duration(milliseconds: 1500), () {
      if (mounted && _copied == which) setState(() => _copied = null);
    });
  }

  void _selectAll(TextEditingController controller, FocusNode focus) {
    focus.requestFocus();
    controller.selection = TextSelection(
      baseOffset: 0,
      extentOffset: controller.text.length,
    );
  }

  @override
  Widget build(BuildContext context) => Container(
    color: Colors.white,
    child: Column(
      children: [
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
          decoration: const BoxDecoration(
            border: Border(bottom: BorderSide(color: Color(0xffeceef1))),
          ),
          child: Row(
            children: [
              const Text(
                '翻译',
                style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
              ),
              if (_targetLang != null) ...[
                const SizedBox(width: 8),
                _chip('→ $_targetLang'),
              ],
              if (_modelName != null) ...[
                const SizedBox(width: 8),
                _chip(_modelName!),
              ],
              const Spacer(),
              _button(_busy ? '翻译中…' : '重新翻译', _busy ? null : _translate),
              const SizedBox(width: 6),
              _button(
                '全选原文',
                _source.isEmpty
                    ? null
                    : () => _selectAll(_sourceController, _sourceFocus),
              ),
              const SizedBox(width: 6),
              _button(
                '全选译文',
                _resultController.text.isEmpty
                    ? null
                    : () => _selectAll(_resultController, _resultFocus),
              ),
              const SizedBox(width: 6),
              _button(
                _copied == 'dst' ? '已复制' : '复制译文',
                _translated == null ? null : () => _copy('dst', _translated!),
              ),
              const SizedBox(width: 6),
              _button('关闭', windowManager.close, ghost: true),
            ],
          ),
        ),
        if (_error != null)
          Container(
            width: double.infinity,
            margin: const EdgeInsets.fromLTRB(16, 12, 16, 0),
            padding: const EdgeInsets.all(10),
            decoration: BoxDecoration(
              color: const Color(0xfffdecec),
              borderRadius: BorderRadius.circular(8),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    _error!,
                    style: const TextStyle(
                      fontSize: 12,
                      color: Color(0xffc0392b),
                    ),
                  ),
                ),
                _button('重试', _translate),
              ],
            ),
          ),
        Expanded(
          child: Row(
            children: [
              _pane('原文', _sourceController, _sourceFocus, 'src'),
              const VerticalDivider(width: 1, color: Color(0xffeceef1)),
              _pane('译文', _resultController, _resultFocus, 'dst'),
            ],
          ),
        ),
        Container(
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 9),
          decoration: const BoxDecoration(
            border: Border(top: BorderSide(color: Color(0xffeceef1))),
          ),
          child: const Text(
            '两边都可以直接用鼠标拖动选择，Ctrl+C 复制；也可以点「全选」一次选完整栏。',
            style: TextStyle(fontSize: 11, color: Color(0xff9aa0ab)),
          ),
        ),
      ],
    ),
  );

  Widget _chip(String label) => Container(
    constraints: const BoxConstraints(maxWidth: 90),
    padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
    decoration: BoxDecoration(
      color: const Color(0xfff0f1f3),
      borderRadius: BorderRadius.circular(10),
    ),
    child: Text(
      label,
      maxLines: 1,
      overflow: TextOverflow.ellipsis,
      style: const TextStyle(fontSize: 11, color: Color(0xff6b7280)),
    ),
  );

  Widget _button(String label, VoidCallback? action, {bool ghost = false}) =>
      TextButton(
        onPressed: action,
        style: TextButton.styleFrom(
          foregroundColor: ghost
              ? const Color(0xff6b7280)
              : const Color(0xff1f2329),
          backgroundColor: ghost ? Colors.transparent : const Color(0xfff0f1f3),
          minimumSize: const Size(0, 28),
          padding: const EdgeInsets.symmetric(horizontal: 11),
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(7)),
        ),
        child: Text(label, style: const TextStyle(fontSize: 12)),
      );

  Widget _pane(
    String title,
    TextEditingController controller,
    FocusNode focus,
    String which,
  ) => Expanded(
    child: Column(
      children: [
        Container(
          height: 33,
          padding: const EdgeInsets.symmetric(horizontal: 14),
          decoration: const BoxDecoration(
            color: Colors.white,
            border: Border(bottom: BorderSide(color: Color(0xfff0f1f3))),
          ),
          child: Row(
            children: [
              Text(
                title,
                style: const TextStyle(fontSize: 12, color: Color(0xff8a919f)),
              ),
              if (which == 'dst' && _busy) ...[
                const SizedBox(width: 8),
                const SizedBox(
                  width: 13,
                  height: 13,
                  child: CircularProgressIndicator(strokeWidth: 2),
                ),
              ],
              const Spacer(),
              TextButton(
                onPressed: controller.text.isEmpty
                    ? null
                    : () => _copy(which, controller.text),
                child: Text(
                  _copied == which ? '已复制' : '复制',
                  style: const TextStyle(fontSize: 12),
                ),
              ),
            ],
          ),
        ),
        Expanded(
          child: SingleChildScrollView(
            padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 12),
            child: SizedBox(
              width: double.infinity,
              child: TextField(
                controller: controller,
                focusNode: focus,
                readOnly: true,
                maxLines: null,
                decoration: InputDecoration(
                  border: InputBorder.none,
                  hintText: which == 'src'
                      ? '（还没有内容）'
                      : _busy
                      ? '翻译中…'
                      : '（还没有译文）',
                  hintStyle: const TextStyle(
                    fontSize: 12,
                    color: Color(0xff9aa0ab),
                  ),
                  isCollapsed: true,
                ),
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
  );
}
