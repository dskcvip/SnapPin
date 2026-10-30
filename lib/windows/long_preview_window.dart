import 'dart:async';
import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

import '../window_roles.dart';
import '../app_log.dart';
import '../window_capture_exclusion.dart';

class LongPreviewWindow extends StatefulWidget {
  const LongPreviewWindow({
    super.key,
    required this.controller,
    required this.initialHeight,
  });
  final WindowController controller;
  final int initialHeight;

  @override
  State<LongPreviewWindow> createState() => _LongPreviewWindowState();
}

class _LongPreviewWindowState extends State<LongPreviewWindow> {
  late int _height = widget.initialHeight;
  String? _thumbPath;
  bool _warning = false;
  String _message = '滚动页面继续截图';
  bool _busy = false;
  late final Future<bool> _exclusionReady;

  @override
  void initState() {
    super.initState();
    _exclusionReady =
        WindowCaptureExclusion.enable(
          'SnapPin Long Preview ${widget.controller.windowId}',
        ).then((value) {
          AppLog.write('long-preview-exclusion', value);
          return value;
        });
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'longProgress') {
        final progress = Map<String, dynamic>.from(call.arguments as Map);
        if (mounted)
          setState(() {
            _height = progress['height'] as int;
            _warning = progress['warning'] as bool;
            _message = progress['message'] as String? ?? _message;
            if (progress['finishing'] is bool) {
              _busy = progress['finishing'] as bool;
            }
            final thumbPath = progress['thumbPath'];
            if (thumbPath is String) _thumbPath = thumbPath;
          });
      } else if (call.method == 'closePreview') {
        await windowManager.hide();
      } else if (call.method == 'prepareCapture') {
        final excluded = await _exclusionReady;
        if (!excluded) await windowManager.hide();
        return !excluded;
      } else if (call.method == 'finishCapture') {
        await windowManager.show();
      } else if (call.method == 'openPreview') {
        final args = Map<String, dynamic>.from(call.arguments as Map);
        if (mounted)
          setState(() {
            _height = args['height'] as int;
            _thumbPath = null;
            _warning = false;
            _message = '滚动页面继续截图';
            _busy = false;
          });
        await windowManager.setPosition(
          Offset((args['x'] as num).toDouble(), (args['y'] as num).toDouble()),
        );
        await windowManager.show();
      }
      return null;
    });
  }

  Future<void> _send(String method) async {
    if (_busy) return;
    setState(() => _busy = true);
    try {
      final ball = await findBallWindow();
      if (ball == null) throw StateError('主窗口不可用');
      await ball.invokeMethod<void>(method);
    } catch (_) {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) => Align(
    alignment: Alignment.bottomCenter,
    child: Container(
      width: 324,
      constraints: const BoxConstraints(maxHeight: 552),
      decoration: BoxDecoration(
        color: Colors.white,
        border: Border.all(color: const Color(0xffc6d0d5)),
        borderRadius: BorderRadius.circular(5),
        boxShadow: const [
          BoxShadow(
            color: Color(0x330a1920),
            blurRadius: 24,
            offset: Offset(0, 8),
          ),
        ],
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            height: 38,
            padding: const EdgeInsets.only(left: 12, right: 8),
            decoration: const BoxDecoration(
              color: Color(0xfff7f9f9),
              border: Border(bottom: BorderSide(color: Color(0xffc6d0d5))),
            ),
            child: Row(
              children: [
                Container(
                  width: 7,
                  height: 7,
                  decoration: const BoxDecoration(
                    color: Color(0xff20a66b),
                    shape: BoxShape.circle,
                  ),
                ),
                const SizedBox(width: 7),
                Text(
                  '$_height px',
                  style: const TextStyle(
                    fontSize: 12,
                    color: Color(0xff34454a),
                  ),
                ),
                const SizedBox(width: 8),
                const Expanded(
                  child: Text(
                    '滚动鼠标选择截图区域',
                    maxLines: 1,
                    style: TextStyle(fontSize: 11, color: Color(0xff54666c)),
                  ),
                ),
                _icon(Icons.close, '取消长截图', () => _send('longCancel')),
                const SizedBox(width: 7),
                _icon(
                  Icons.check,
                  '完成长截图',
                  () => _send('longStop'),
                  color: const Color(0xff087b50),
                ),
              ],
            ),
          ),
          if (_thumbPath != null)
            Flexible(
              child: Image.file(
                File(_thumbPath!),
                fit: BoxFit.contain,
                gaplessPlayback: true,
              ),
            ),
          if (_warning || _message.isNotEmpty)
            Container(
              width: double.infinity,
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
              color: _warning
                  ? const Color(0xfffff2e8)
                  : const Color(0xfff7f9f9),
              child: Text(
                _message,
                style: TextStyle(
                  fontSize: 11,
                  color: _warning
                      ? const Color(0xff9b3d18)
                      : const Color(0xff54666c),
                ),
              ),
            ),
        ],
      ),
    ),
  );

  Widget _icon(
    IconData icon,
    String label,
    VoidCallback action, {
    Color color = const Color(0xff54666c),
  }) => Tooltip(
    message: label,
    child: IconButton(
      onPressed: _busy ? null : action,
      icon: Icon(icon, size: 17, color: color),
      constraints: const BoxConstraints.tightFor(width: 26, height: 26),
      padding: EdgeInsets.zero,
      splashRadius: 14,
    ),
  );
}
