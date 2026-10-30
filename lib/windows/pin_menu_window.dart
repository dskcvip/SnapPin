import 'dart:async';
import 'dart:convert';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import '../app_log.dart';

class PinMenuWindow extends StatefulWidget {
  const PinMenuWindow({
    super.key,
    required this.controller,
    required this.targetId,
  });

  final WindowController controller;
  final String targetId;

  @override
  State<PinMenuWindow> createState() => _PinMenuWindowState();
}

class _PinMenuWindowState extends State<PinMenuWindow> with WindowListener {
  WindowController? _target;
  Map<String, dynamic>? _flags;
  List<String> _groups = ['默认'];
  String? _submenu;
  bool _closing = false;
  bool _acting = false;
  bool _sliding = false;
  double _opacity = 1;

  @override
  void initState() {
    super.initState();
    windowManager.addListener(this);
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'closeSelf') {
        unawaited(Future<void>.delayed(Duration.zero, _close));
      }
      return null;
    });
    _load();
  }

  Future<void> _load() async {
    final windows = await WindowController.getAll();
    for (final window in windows) {
      if (window.windowId == widget.targetId) _target = window;
    }
    final target = _target;
    if (target == null) {
      await _close();
      return;
    }
    final raw = await target.invokeMethod<String>('getPinFlags');
    final groups = <String>{'默认'};
    for (final window in windows) {
      if (window.arguments.isEmpty) continue;
      if ((jsonDecode(window.arguments) as Map)['role'] != 'pin') continue;
      try {
        final info = await window.invokeMethod<String>('getPinInfo');
        if (info != null) {
          groups.add((jsonDecode(info) as Map)['group'] as String);
        }
      } catch (_) {}
    }
    if (mounted && raw != null) {
      setState(() {
        _flags = Map<String, dynamic>.from(jsonDecode(raw) as Map);
        _opacity = (_flags!['opacity'] as num).toDouble();
        _groups = groups.toList()..sort();
      });
    }
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    super.dispose();
  }

  @override
  void onWindowBlur() {
    if (!_acting && !_sliding) _close();
  }

  Future<void> _close() async {
    if (_closing) return;
    _closing = true;
    await windowManager.close();
  }

  Future<void> _action(String id) async {
    final target = _target;
    if (target == null) return;
    _acting = true;
    await windowManager.hide();
    try {
      if (id.startsWith('group:')) {
        await target.invokeMethod<void>('setPinGroup', id.substring(6));
      } else if (id == 'group-only' ||
          id == 'group-hide' ||
          id == 'group-show-all') {
        final group = _flags?['group'] as String? ?? '默认';
        for (final window in await WindowController.getAll()) {
          if (window.arguments.isEmpty) continue;
          if ((jsonDecode(window.arguments) as Map)['role'] != 'pin') continue;
          if (id == 'group-show-all') {
            await window.invokeMethod<void>('showPin');
            continue;
          }
          final info = await window.invokeMethod<String>('getPinInfo');
          if (info == null) continue;
          final same = (jsonDecode(info) as Map)['group'] == group;
          final show = id == 'group-only' ? same : !same;
          await window.invokeMethod<void>(show ? 'showPin' : 'hidePin');
        }
      } else {
        await target.invokeMethod<void>('pinAction', id);
      }
    } finally {
      await _close();
    }
  }

  @override
  Widget build(BuildContext context) {
    final flags = _flags;
    return Focus(
      autofocus: true,
      onKeyEvent: (_, event) {
        if (event is KeyDownEvent &&
            event.logicalKey == LogicalKeyboardKey.escape) {
          _close();
          return KeyEventResult.handled;
        }
        return KeyEventResult.ignored;
      },
      child: flags == null
          ? const SizedBox.shrink()
          : Stack(
              children: [
                Positioned.fill(
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: _close,
                  ),
                ),
                Positioned(
                  left: 18,
                  top: 18,
                  child: _panel(
                    width: 200,
                    children: [
                      Padding(
                        padding: const EdgeInsets.fromLTRB(10, 3, 10, 0),
                        child: Row(
                          children: [
                            const Text(
                              '不透明度',
                              style: TextStyle(
                                fontSize: 13.5,
                                color: Color(0xff1f2933),
                              ),
                            ),
                            const Spacer(),
                            Text(
                              '${(_opacity * 100).round()}%',
                              style: const TextStyle(
                                fontSize: 12,
                                color: Color(0x730f172a),
                              ),
                            ),
                          ],
                        ),
                      ),
                      Padding(
                        padding: const EdgeInsets.fromLTRB(10, 4, 10, 7),
                        child: SliderTheme(
                          data: SliderTheme.of(context).copyWith(
                            trackHeight: 4,
                            activeTrackColor: const Color(0xff1f2933),
                            inactiveTrackColor: const Color(0x240f172a),
                            trackShape: const _MenuSliderTrack(),
                            thumbColor: Colors.white,
                            thumbShape: const _MenuSliderThumb(),
                            overlayShape: SliderComponentShape.noOverlay,
                          ),
                          child: SizedBox(
                            height: 14,
                            child: Slider(
                              value: _opacity,
                              min: .15,
                              max: 1,
                              divisions: 17,
                              onChangeStart: (_) => _sliding = true,
                              onChangeEnd: (value) {
                                _sliding = false;
                                AppLog.write(
                                  'pin-menu-opacity',
                                  'window=${widget.targetId} value=$value',
                                );
                              },
                              onChanged: (value) {
                                setState(() => _opacity = value);
                                _target?.invokeMethod<void>(
                                  'setPinOpacity',
                                  value,
                                );
                              },
                            ),
                          ),
                        ),
                      ),
                      const Divider(height: 9),
                      _item('分组', submenu: 'group'),
                      _item('图像处理', submenu: 'image'),
                      _item('复制图像', action: 'copy', root: true),
                      _item('保存图像', action: 'save', root: true),
                      const Divider(height: 9),
                      _item('鼠标穿透', action: 'pass-through', root: true),
                      const Divider(height: 9),
                      _item(
                        '边框阴影',
                        action: 'shadow',
                        checked: flags['shadow'] as bool,
                        hint: 'Y',
                        root: true,
                      ),
                      _item(
                        '窗口置顶',
                        action: 'topmost',
                        checked: flags['topmost'] as bool,
                        hint: 'T',
                        root: true,
                      ),
                      _item(
                        '锁定',
                        action: 'lock',
                        checked: flags['locked'] as bool,
                        hint: 'L',
                        root: true,
                      ),
                      _item(
                        '工具栏',
                        action: 'toolbar',
                        checked: flags['toolbar'] as bool,
                        hint: 'H',
                        root: true,
                      ),
                      const Divider(height: 9),
                      _item('关闭', action: 'close', hint: 'Esc', root: true),
                    ],
                  ),
                ),
                if (_submenu == 'group')
                  Positioned(
                    left: 225,
                    top: 90,
                    child: _panel(
                      width: 190,
                      children: [
                        for (final group in _groups)
                          _item(
                            group,
                            action: 'group:$group',
                            checked: group == flags['group'],
                          ),
                        const Divider(height: 9),
                        _item(
                          '只看「${flags['group'] ?? '默认'}」组',
                          action: 'group-only',
                        ),
                        _item(
                          '隐藏「${flags['group'] ?? '默认'}」组',
                          action: 'group-hide',
                        ),
                        _item('显示全部贴图', action: 'group-show-all'),
                      ],
                    ),
                  ),
                if (_submenu == 'image' ||
                    _submenu == 'alpha' ||
                    _submenu == 'brightness')
                  Positioned(
                    left: 225,
                    top: 124,
                    child: _panel(
                      width: 160,
                      children: [
                        _item(
                          '向右旋转',
                          action: 'rotate-right',
                          hint: '1',
                          parentMenu: 'image',
                        ),
                        _item(
                          '向左旋转',
                          action: 'rotate-left',
                          hint: '2',
                          parentMenu: 'image',
                        ),
                        const Divider(height: 9),
                        _item(
                          '水平翻转',
                          action: 'flip-h',
                          checked: flags['flipH'] as bool,
                          hint: '3',
                          parentMenu: 'image',
                        ),
                        _item(
                          '垂直翻转',
                          action: 'flip-v',
                          checked: flags['flipV'] as bool,
                          hint: '4',
                          parentMenu: 'image',
                        ),
                        const Divider(height: 9),
                        _item(
                          '灰度化',
                          action: 'grayscale',
                          checked: flags['gray'] as bool,
                          hint: '5',
                          parentMenu: 'image',
                        ),
                        _item(
                          '反色',
                          action: 'invert',
                          checked: flags['invert'] as bool,
                          hint: '6',
                          parentMenu: 'image',
                        ),
                        const Divider(height: 9),
                        _item('图像不透明度', submenu: 'alpha'),
                        _item('调整亮度', submenu: 'brightness'),
                        const Divider(height: 9),
                        _item(
                          '重置所有处理',
                          action: 'reset-edit',
                          hint: '0',
                          parentMenu: 'image',
                        ),
                      ],
                    ),
                  ),
                if (_submenu == 'alpha')
                  Positioned(
                    left: 392,
                    top: 361,
                    child: _panel(
                      width: 160,
                      children: [
                        for (final value in [100, 75, 50, 25])
                          _item(
                            '$value%',
                            action: 'alpha-$value',
                            checked:
                                ((flags['alpha'] as num) * 100).round() ==
                                value,
                          ),
                      ],
                    ),
                  ),
                if (_submenu == 'brightness')
                  Positioned(
                    left: 392,
                    top: 395,
                    child: _panel(
                      width: 160,
                      children: [
                        _item('变亮', action: 'bright-up', reserveCheck: false),
                        _item('变暗', action: 'bright-down', reserveCheck: false),
                        _item(
                          '重置亮度',
                          action: 'bright-reset',
                          reserveCheck: false,
                        ),
                        const Divider(height: 9),
                        _item(
                          '当前 ${((flags['brightness'] as num) * 100).round()}%',
                          reserveCheck: false,
                        ),
                      ],
                    ),
                  ),
              ],
            ),
    );
  }

  Widget _panel({required double width, required List<Widget> children}) =>
      Container(
        width: width,
        padding: const EdgeInsets.all(6),
        decoration: BoxDecoration(
          color: Colors.white,
          border: Border.all(color: const Color(0x140f172a)),
          borderRadius: BorderRadius.circular(12),
          boxShadow: const [
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
        child: Column(mainAxisSize: MainAxisSize.min, children: children),
      );

  Widget _item(
    String label, {
    String? action,
    String? submenu,
    bool? checked,
    String? hint,
    bool root = false,
    String? parentMenu,
    bool reserveCheck = true,
  }) => SizedBox(
    height: 34,
    child: MouseRegion(
      onEnter: (_) {
        if (submenu != null) setState(() => _submenu = submenu);
        if (root && _submenu != null) setState(() => _submenu = null);
        if (parentMenu != null && _submenu != parentMenu) {
          setState(() => _submenu = parentMenu);
        }
      },
      child: Material(
        color: Colors.transparent,
        child: InkWell(
          hoverColor: const Color(0x100f172a),
          onTap: () {
            if (submenu != null) {
              setState(() => _submenu = submenu);
            } else if (action != null) {
              _action(action);
            }
          },
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: Row(
              children: [
                if (reserveCheck) ...[
                  SizedBox(
                    width: 16,
                    child: checked == true
                        ? const Icon(Icons.check, size: 14)
                        : null,
                  ),
                  const SizedBox(width: 12),
                ],
                Expanded(
                  child: Text(
                    label,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 13.5,
                      color: Color(0xff1f2933),
                    ),
                  ),
                ),
                if (submenu != null)
                  const Icon(Icons.chevron_right, size: 15)
                else if (hint != null)
                  Text(
                    hint,
                    style: const TextStyle(
                      fontSize: 11,
                      color: Color(0xff9299a3),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

class _MenuSliderTrack extends RoundedRectSliderTrackShape {
  const _MenuSliderTrack();

  @override
  Rect getPreferredRect({
    required RenderBox parentBox,
    Offset offset = Offset.zero,
    required SliderThemeData sliderTheme,
    bool isEnabled = false,
    bool isDiscrete = false,
  }) => Rect.fromLTWH(
    offset.dx + 6.5,
    offset.dy + (parentBox.size.height - 4) / 2,
    parentBox.size.width - 13,
    4,
  );
}

class _MenuSliderThumb extends SliderComponentShape {
  const _MenuSliderThumb();

  @override
  Size getPreferredSize(bool isEnabled, bool isDiscrete) => const Size(13, 13);

  @override
  void paint(
    PaintingContext context,
    Offset center, {
    required Animation<double> activationAnimation,
    required Animation<double> enableAnimation,
    required bool isDiscrete,
    required TextPainter labelPainter,
    required RenderBox parentBox,
    required SliderThemeData sliderTheme,
    required TextDirection textDirection,
    required double value,
    required double textScaleFactor,
    required Size sizeWithOverflow,
  }) {
    final canvas = context.canvas;
    canvas.drawCircle(
      center + const Offset(0, 1),
      6.5,
      Paint()
        ..color = const Color(0x400f172a)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 2),
    );
    canvas.drawCircle(center, 6.5, Paint()..color = Colors.white);
    canvas.drawCircle(
      center,
      6.5,
      Paint()
        ..color = const Color(0x590f172a)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }
}
