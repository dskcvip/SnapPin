import 'dart:async';
import 'dart:io';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import '../app_log.dart';
import 'app_menu_metrics.dart';
import '../window_roles.dart';

class AppMenuWindow extends StatefulWidget {
  const AppMenuWindow({
    super.key,
    required this.controller,
    this.initialWarnings = const [],
  });

  final WindowController controller;
  final List<String> initialWarnings;

  @override
  State<AppMenuWindow> createState() => _AppMenuWindowState();
}

class _AppMenuWindowState extends State<AppMenuWindow> with WindowListener {
  bool _open = false;
  String _captureShortcut = 'Ctrl+1';
  String _pinShortcut = 'Ctrl+2';
  List<String> _warnings = const [];

  @override
  void initState() {
    super.initState();
    _warnings = widget.initialWarnings;
    windowManager.addListener(this);
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'closeSelf') {
        unawaited(Future<void>.delayed(Duration.zero, _close));
      } else if (call.method == 'openAt') {
        final point = Map<String, dynamic>.from(call.arguments as Map);
        final warnings =
            (point['warnings'] as List?)?.cast<String>() ?? const <String>[];
        setState(() {
          _captureShortcut = point['captureShortcut'] as String? ?? 'Ctrl+1';
          _pinShortcut = point['pinShortcut'] as String? ?? 'Ctrl+2';
          _warnings = warnings;
        });
        await windowManager.setSize(
          Size(appMenuWidth, appMenuHeight(warnings)),
        );
        await windowManager.setPosition(
          Offset(
            (point['x'] as num).toDouble(),
            (point['y'] as num).toDouble(),
          ),
        );
        await windowManager.show();
        await windowManager.focus();
        _open = true;
      } else if (Platform.isMacOS &&
          Platform.environment['SNAPPIN_STARTUP_SMOKE'] == '1') {
        if (call.method == 'smokeInspect') {
          await WidgetsBinding.instance.endOfFrame;
          return _open &&
              await windowManager.isVisible() &&
              await windowManager.isFocused();
        }
        if (call.method == 'smokeToggleBall') await _action('toggle-ball');
        if (call.method == 'smokeQuit') {
          unawaited(_action('quit'));
        }
      }
      return null;
    });
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    super.dispose();
  }

  @override
  void onWindowBlur() => unawaited(
    Future<void>.delayed(const Duration(milliseconds: 180), () async {
      if (_open && !await windowManager.isFocused()) await _close();
    }),
  );

  Future<void> _close() async {
    if (!_open) return;
    _open = false;
    await windowManager.hide();
  }

  Future<void> _action(String id) async {
    AppLog.write('menu-action', id);
    await _close();
    try {
      final ball = await findBallWindow();
      if (ball == null) throw StateError('Ball window not found');
      await ball.invokeMethod<void>('menuAction', id);
    } catch (error, stack) {
      AppLog.write('menu-action-failed', '$id: $error\n$stack');
      rethrow;
    }
  }

  @override
  Widget build(BuildContext context) => Focus(
    autofocus: true,
    onKeyEvent: (_, event) {
      if (event is KeyDownEvent &&
          event.logicalKey == LogicalKeyboardKey.escape) {
        _close();
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    },
    child: Stack(
      children: [
        Positioned.fill(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: _close,
          ),
        ),
        Padding(
          padding: const EdgeInsets.all(18),
          child: Container(
            decoration: BoxDecoration(
              color: Colors.white,
              borderRadius: BorderRadius.circular(12),
              boxShadow: const [
                BoxShadow(color: Color(0x140f172a), spreadRadius: 1),
                BoxShadow(
                  color: Color(0x2e0f172a),
                  blurRadius: 16,
                  offset: Offset(0, 6),
                ),
                BoxShadow(
                  color: Color(0x1f0f172a),
                  blurRadius: 3,
                  offset: Offset(0, 1),
                ),
              ],
            ),
            padding: const EdgeInsets.all(6),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                if (_warnings.isNotEmpty)
                  Container(
                    height: appMenuWarningHeight,
                    alignment: Alignment.centerLeft,
                    padding: const EdgeInsets.symmetric(horizontal: 10),
                    child: Text(
                      appMenuWarningText(_warnings),
                      softWrap: true,
                      style: const TextStyle(
                        fontSize: 12,
                        height: 1.4,
                        color: Color(0xffc2410c),
                      ),
                    ),
                  ),
                _item('截图', _captureShortcut, () => _action('capture')),
                _item('剪贴板贴图', _pinShortcut, () => _action('pin-clipboard')),
                _item('显示 / 隐藏悬浮球', '', () => _action('toggle-ball')),
                const Divider(height: 13, indent: 8, endIndent: 8),
                _item('贴图管理...', '', () => _action('pin-manager')),
                _item('设置', '', () => _action('settings')),
                _item('打开运行日志', '', () => _action('open-log')),
                const Divider(height: 13, indent: 8, endIndent: 8),
                _item('退出', '', () => _action('quit')),
              ],
            ),
          ),
        ),
      ],
    ),
  );

  Widget _item(String label, String shortcut, VoidCallback action) => SizedBox(
    height: 34,
    child: Material(
      color: Colors.transparent,
      child: InkWell(
        onTap: action,
        borderRadius: BorderRadius.circular(7),
        hoverColor: const Color(0x0f0f172a),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 10),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  label,
                  style: const TextStyle(
                    fontSize: 13.5,
                    color: Color(0xff1f2933),
                  ),
                ),
              ),
              if (shortcut.isNotEmpty)
                Text(
                  shortcut,
                  style: const TextStyle(
                    fontSize: 12,
                    color: Color(0xff9299a3),
                  ),
                ),
            ],
          ),
        ),
      ),
    ),
  );
}
