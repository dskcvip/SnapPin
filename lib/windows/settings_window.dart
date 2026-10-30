import 'dart:convert';
import 'dart:io';
import 'dart:ffi' show Abi;

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:window_manager/window_manager.dart';

import '../app_config.dart';
import '../app_log.dart';
import '../window_roles.dart';
import '../shortcuts.dart';
import 'models_settings.dart';
import 'settings_icons.dart';

class SettingsWindow extends StatefulWidget {
  const SettingsWindow({
    super.key,
    required this.controller,
    this.initialConfig,
  });
  final WindowController controller;
  final AppConfig? initialConfig;

  @override
  State<SettingsWindow> createState() => _SettingsWindowState();
}

class _SettingsWindowState extends State<SettingsWindow> with WindowListener {
  AppConfig? _config;
  AppConfig? _savedConfig;
  String _page = '通用';
  String _keyQuery = '';
  String? _recording;
  String? _requestedRecording;
  int _recordingRevision = 0;
  bool _disposing = false;
  Future<void> _recordingOperation = Future<void>.value();
  bool _saving = false;
  bool _dirty = false;
  bool _closeArmed = false;
  bool _closing = false;
  String? _saveError;
  int _editRevision = 0;

  @override
  void initState() {
    super.initState();
    windowManager.addListener(this);
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'focusSettings') {
        setState(() => _closeArmed = false);
        await windowManager.show();
        await windowManager.focus();
      }
      return null;
    });
    (widget.initialConfig == null
            ? ConfigStore.load()
            : Future.value(widget.initialConfig!))
        .then((config) {
          if (mounted)
            setState(() {
              _config = config;
              _savedConfig = config;
            });
        });
  }

  @override
  void dispose() {
    _disposing = true;
    _setRecording(null);
    windowManager.removeListener(this);
    super.dispose();
  }

  @override
  void onWindowBlur() => _setRecording(null);

  @override
  void onWindowMinimize() => _setRecording(null);

  void _setRecording(String? field) {
    if (field == null && _requestedRecording == null && _recording == null)
      return;
    _requestedRecording = field;
    final revision = ++_recordingRevision;
    if (field == null && mounted && !_disposing)
      setState(() => _recording = null);
    _recordingOperation = _recordingOperation
        .catchError((Object error) {
          AppLog.write('settings-recording-failed', error);
        })
        .then((_) async {
          if (field != null &&
              (!mounted || _disposing || revision != _recordingRevision))
            return;
          final ball = await findBallWindow();
          if (field != null &&
              (!mounted || _disposing || revision != _recordingRevision))
            return;
          await ball?.invokeMethod<void>(
            field == null ? 'resumeHotkeys' : 'suspendHotkeys',
          );
          if (field != null &&
              mounted &&
              !_disposing &&
              revision == _recordingRevision) {
            setState(() => _recording = field);
          }
          AppLog.write('settings-recording', field ?? 'stopped');
        })
        .catchError((Object error) async {
          _requestedRecording = null;
          if (mounted && !_disposing) setState(() => _recording = null);
          AppLog.write('settings-recording-failed', error);
          try {
            final ball = await findBallWindow();
            await ball?.invokeMethod<void>('resumeHotkeys');
          } catch (resumeError) {
            AppLog.write('settings-hotkeys-resume-failed', resumeError);
          }
        });
  }

  @override
  void onWindowClose() => _requestClose();

  Future<void> _requestClose() async {
    _setRecording(null);
    if (_closing || _saving) return;
    AppLog.write('settings-close-request', 'dirty=$_dirty armed=$_closeArmed');
    if (_dirty && !_closeArmed) {
      setState(() => _closeArmed = true);
      return;
    }
    await _discardAndClose();
  }

  Future<void> _discardAndClose() async {
    if (_closing) return;
    _closing = true;
    _setRecording(null);
    await _recordingOperation;
    if (_dirty && _savedConfig != null) {
      try {
        await _preview(_savedConfig!);
      } catch (error) {
        AppLog.write('settings-restore-preview-failed', error);
      }
    }
    if (mounted) {
      setState(() {
        _config = _savedConfig;
        _dirty = false;
        _closeArmed = false;
        _saveError = null;
      });
    }
    await windowManager.hide();
    AppLog.write('settings-hidden');
    _closing = false;
  }

  Future<void> _saveAndClose() async {
    if (await _save() && !_dirty) {
      await _discardAndClose();
    }
  }

  Future<void> _preview(AppConfig config) async {
    final ball = await findBallWindow();
    await ball?.invokeMethod<void>(
      'applySettings',
      jsonEncode(config.toJson()),
    );
  }

  Future<void> _previewBall(AppConfig config) async {
    final ball = await findBallWindow();
    await ball?.invokeMethod<void>(
      'previewBall',
      jsonEncode({'size': config.ballSize, 'opacity': config.ballOpacity}),
    );
  }

  void _patch(AppConfig config, {bool previewBall = false}) {
    ++_editRevision;
    setState(() {
      _config = config;
      _dirty = true;
      _closeArmed = false;
      _saveError = null;
    });
    if (previewBall) {
      _previewBall(config).catchError((Object error) {
        AppLog.write('settings-preview-failed', error);
      });
    }
  }

  Future<bool> _save() async {
    final initialConfig = _config;
    if (initialConfig == null || _saving) return false;
    AppConfig config = initialConfig;
    final revision = _editRevision;
    setState(() => _saving = true);
    try {
      final ball = await findBallWindow();
      try {
        final center = await ball?.invokeMethod<Map>('getBallCenter');
        if (center != null) {
          config = config.copyWith(
            ballCenter: Offset(
              (center['x'] as num).toDouble(),
              (center['y'] as num).toDouble(),
            ),
          );
        }
      } catch (error) {
        AppLog.write('settings-ball-position-failed', error);
      }
      await ConfigStore.save(config);
      try {
        await _preview(config);
      } catch (error) {
        AppLog.write('settings-apply-failed', error);
      }
      if (mounted)
        setState(() {
          _savedConfig = config;
          if (_editRevision == revision) {
            _config = config;
            _dirty = false;
          }
          _closeArmed = false;
        });
      return true;
    } catch (error) {
      if (mounted) setState(() => _saveError = '保存失败：$error');
      return false;
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  Widget _saveButton() => Align(
    alignment: Alignment.center,
    child: FilledButton(
      onPressed: !_dirty || _saving ? null : () => _save(),
      child: Text(
        _saving
            ? '保存中…'
            : _dirty
            ? '保存设置'
            : '已保存',
      ),
    ),
  );

  Future<void> _pickThemeColor(AppConfig config) async {
    var hsv = HSVColor.fromColor(config.color);
    final hex = TextEditingController(text: config.themeColor.toUpperCase());
    var valid = true;
    final selected = await showDialog<String>(
      context: context,
      barrierColor: const Color(0x520f172a),
      builder: (context) => StatefulBuilder(
        builder: (context, update) {
          void change(HSVColor next) {
            update(() {
              hsv = next;
              hex.text =
                  '#${(next.toColor().toARGB32() & 0xffffff).toRadixString(16).padLeft(6, '0').toUpperCase()}';
              valid = true;
            });
          }

          return AlertDialog(
            title: const Text('主题色', style: TextStyle(fontSize: 16)),
            content: SizedBox(
              width: 300,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Container(
                    height: 48,
                    decoration: BoxDecoration(
                      color: hsv.toColor(),
                      borderRadius: BorderRadius.circular(6),
                    ),
                  ),
                  const SizedBox(height: 12),
                  _colorSlider(
                    '色相',
                    hsv.hue,
                    360,
                    (value) => change(hsv.withHue(value)),
                  ),
                  _colorSlider(
                    '饱和度',
                    hsv.saturation,
                    1,
                    (value) => change(hsv.withSaturation(value)),
                  ),
                  _colorSlider(
                    '亮度',
                    hsv.value,
                    1,
                    (value) => change(hsv.withValue(value)),
                  ),
                  TextField(
                    controller: hex,
                    maxLength: 7,
                    decoration: InputDecoration(
                      labelText: '十六进制颜色',
                      hintText: '#12A5D8',
                      errorText: valid ? null : '请输入 #RRGGBB',
                      counterText: '',
                    ),
                    onChanged: (value) {
                      final parsed = RegExp(r'^#[0-9a-fA-F]{6}$')
                          .hasMatch(value);
                      update(() {
                        valid = parsed;
                        if (parsed) {
                          hsv = HSVColor.fromColor(
                            Color(
                              0xff000000 |
                                  int.parse(value.substring(1), radix: 16),
                            ),
                          );
                        }
                      });
                    },
                  ),
                ],
              ),
            ),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(context),
                child: const Text('取消'),
              ),
              FilledButton(
                onPressed: valid
                    ? () => Navigator.pop(context, hex.text.toUpperCase())
                    : null,
                child: const Text('确定'),
              ),
            ],
          );
        },
      ),
    );
    hex.dispose();
    if (selected != null && mounted)
      _patch(config.copyWith(themeColor: selected));
  }

  Widget _colorSlider(
    String label,
    double value,
    double max,
    ValueChanged<double> onChanged,
  ) => Row(
    children: [
      SizedBox(
        width: 52,
        child: Text(label, style: const TextStyle(fontSize: 13)),
      ),
      Expanded(
        child: Slider(value: value, max: max, onChanged: onChanged),
      ),
    ],
  );

  @override
  Widget build(BuildContext context) {
    final config = _config;
    if (config == null) return const Center(child: CircularProgressIndicator());
    return Theme(
      data: Theme.of(context).copyWith(
        filledButtonTheme: FilledButtonThemeData(
          style: FilledButton.styleFrom(
            backgroundColor: config.color,
            foregroundColor: Colors.white,
            minimumSize: const Size(0, 32),
            padding: const EdgeInsets.symmetric(horizontal: 14),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(8),
            ),
            textStyle: Theme.of(context).textTheme.bodyMedium!
                .copyWith(fontSize: 13),
            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
            disabledBackgroundColor: config.color.withValues(alpha: .55),
            disabledForegroundColor: Colors.white,
          ),
        ),
        outlinedButtonTheme: OutlinedButtonThemeData(
          style: OutlinedButton.styleFrom(
            backgroundColor: const Color(0xfff0f1f3),
            foregroundColor: const Color(0xff1f2329),
            side: BorderSide.none,
            minimumSize: const Size(0, 32),
            padding: const EdgeInsets.symmetric(horizontal: 14),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(8),
            ),
            textStyle: Theme.of(context).textTheme.bodyMedium!
                .copyWith(fontSize: 13),
            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
          ),
        ),
        textButtonTheme: TextButtonThemeData(
          style: TextButton.styleFrom(
            backgroundColor: const Color(0xfff0f1f3),
            foregroundColor: const Color(0xff1f2329),
            minimumSize: const Size(0, 32),
            padding: const EdgeInsets.symmetric(horizontal: 14),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(8),
            ),
            textStyle: Theme.of(context).textTheme.bodyMedium!
                .copyWith(fontSize: 13),
            tapTargetSize: MaterialTapTargetSize.shrinkWrap,
          ),
        ),
      ),
      child: Focus(
        autofocus: true,
        onKeyEvent: (_, event) {
          if (event is! KeyDownEvent) {
            return KeyEventResult.ignored;
          }
          if (_recording == null) {
            if (event.logicalKey == LogicalKeyboardKey.escape) {
              _requestClose();
              return KeyEventResult.handled;
            }
            return KeyEventResult.ignored;
          }
          if (event.logicalKey == LogicalKeyboardKey.escape) {
            _setRecording(null);
            return KeyEventResult.handled;
          }
          final keyboard = HardwareKeyboard.instance;
          final modifiers = <String>[
            if (keyboard.isControlPressed) 'Ctrl',
            if (keyboard.isMetaPressed) 'Super',
            if (keyboard.isAltPressed) 'Alt',
            if (keyboard.isShiftPressed) 'Shift',
          ];
          if (modifiers.isEmpty) return KeyEventResult.handled;
          final name = shortcutKeyLabel(event);
          if (name == null) return KeyEventResult.handled;
          final shortcut = [...modifiers, name].join('+');
          _patch(
            _recording == 'capture'
                ? config.copyWith(shortcutCapture: shortcut)
                : config.copyWith(shortcutPinClipboard: shortcut),
          );
          _setRecording(null);
          return KeyEventResult.handled;
        },
        child: Stack(
          children: [
            Row(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Container(
                  width: 208,
                  color: const Color(0xfff7f8fa),
                  padding: const EdgeInsets.fromLTRB(10, 12, 10, 10),
                  child: Column(
                    children: [
                      const SizedBox(height: 8),
                      const Align(
                        alignment: Alignment.centerLeft,
                        child: Padding(
                          padding: EdgeInsets.fromLTRB(8, 0, 0, 2),
                          child: Text(
                            '设置',
                            style: TextStyle(
                              fontSize: 12,
                              color: Color(0xff9aa0ab),
                            ),
                          ),
                        ),
                      ),
                      for (final item in const [
                        ('通用', Icons.settings_outlined),
                        ('快捷键', Icons.keyboard_outlined),
                        ('模型', Icons.widgets_outlined),
                      ])
                        _navItem(item.$1, item.$2),
                      const Align(
                        alignment: Alignment.centerLeft,
                        child: Padding(
                          padding: EdgeInsets.fromLTRB(8, 14, 0, 2),
                          child: Text(
                            '数据与安全',
                            style: TextStyle(
                              fontSize: 12,
                              color: Color(0xff9aa0ab),
                            ),
                          ),
                        ),
                      ),
                      _navItem('数据管理', Icons.storage_outlined),
                      const SizedBox(height: 4),
                      _navItem('关于', Icons.info_outline),
                    ],
                  ),
                ),
                const VerticalDivider(width: 1, color: Color(0xffeceef1)),
                Expanded(
                  child: ColoredBox(
                    color: _page == '关于'
                        ? const Color(0xfff7f7f7)
                        : Colors.white,
                    child: SingleChildScrollView(
                      padding: _page == '关于'
                          ? const EdgeInsets.fromLTRB(30, 40, 30, 28)
                          : const EdgeInsets.fromLTRB(24, 20, 24, 28),
                      child: switch (_page) {
                        '通用' => _general(config),
                        '快捷键' => _shortcuts(config),
                        '模型' => Column(
                          children: [
                            ModelsSettings(
                              config: config,
                              savedConfig: _savedConfig,
                              onChanged: _patch,
                            ),
                            const SizedBox(height: 18),
                            _saveButton(),
                          ],
                        ),
                        '数据管理' => _dataPage(),
                        _ => _aboutPage(),
                      },
                    ),
                  ),
                ),
              ],
            ),
            if (_closeArmed)
              Positioned(
                left: 0,
                right: 0,
                bottom: 18,
                child: Center(
                  child: Container(
                    padding: const EdgeInsets.symmetric(
                      horizontal: 16,
                      vertical: 9,
                    ),
                    decoration: BoxDecoration(
                      color: const Color(0xfffff6e6),
                      border: Border.all(color: const Color(0xfff0c98a)),
                      borderRadius: BorderRadius.circular(9),
                      boxShadow: const [
                        BoxShadow(
                          color: Color(0x2410172a),
                          blurRadius: 18,
                          offset: Offset(0, 6),
                        ),
                      ],
                    ),
                    child: Wrap(
                      spacing: 8,
                      runSpacing: 6,
                      crossAxisAlignment: WrapCrossAlignment.center,
                      children: [
                        const Text(
                          '有未保存的修改',
                          style: TextStyle(
                            fontSize: 13,
                            color: Color(0xff7a3b00),
                          ),
                        ),
                        FilledButton(
                          onPressed: _saveAndClose,
                          style: FilledButton.styleFrom(
                            minimumSize: const Size(0, 26),
                            padding: const EdgeInsets.symmetric(horizontal: 11),
                            textStyle: Theme.of(context).textTheme.bodyMedium!
                                .copyWith(fontSize: 12),
                            shape: RoundedRectangleBorder(
                              borderRadius: BorderRadius.circular(6),
                            ),
                          ),
                          child: const Text('保存并关闭'),
                        ),
                        OutlinedButton(
                          onPressed: _discardAndClose,
                          style: OutlinedButton.styleFrom(
                            backgroundColor: Colors.white,
                            foregroundColor: const Color(0xff7a3b00),
                            side: const BorderSide(color: Color(0xffe6c48c)),
                            minimumSize: const Size(0, 26),
                            padding: const EdgeInsets.symmetric(horizontal: 11),
                            textStyle: Theme.of(context).textTheme.bodyMedium!
                                .copyWith(fontSize: 12),
                            shape: RoundedRectangleBorder(
                              borderRadius: BorderRadius.circular(6),
                            ),
                          ),
                          child: const Text('放弃并关闭'),
                        ),
                        TextButton(
                          onPressed: () => setState(() => _closeArmed = false),
                          style: TextButton.styleFrom(
                            backgroundColor: Colors.transparent,
                            foregroundColor: const Color(0x8c0f172a),
                            minimumSize: const Size(0, 26),
                            padding: const EdgeInsets.symmetric(horizontal: 11),
                            textStyle: Theme.of(context).textTheme.bodyMedium!
                                .copyWith(fontSize: 12),
                          ),
                          child: const Text('取消'),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            if (_saveError != null)
              Positioned(
                left: 224,
                right: 16,
                bottom: _closeArmed ? 64 : 8,
                child: Text(
                  _saveError!,
                  style: const TextStyle(color: Colors.red, fontSize: 12),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Widget _general(AppConfig config) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      _heading('通用'),
      const Padding(
        padding: EdgeInsets.only(bottom: 16),
        child: Text(
          '外观、悬浮球与翻译目标语言等常规项。',
          style: TextStyle(fontSize: 12, height: 1.6, color: Color(0xff8a919f)),
        ),
      ),
      _section('', [
        Row(
          children: [
            const SizedBox(
              width: 120,
              child: Text(
                '主题色',
                style: TextStyle(fontSize: 13, color: Color(0xff4e5566)),
              ),
            ),
            const SizedBox(width: 14),
            Tooltip(
              message: '选择主题色',
              child: InkWell(
                onTap: () => _pickThemeColor(config),
                child: Container(
                  width: 44,
                  height: 28,
                  padding: const EdgeInsets.all(2),
                  decoration: BoxDecoration(
                    border: Border.all(color: const Color(0xffe2e4e8)),
                    borderRadius: BorderRadius.circular(6),
                  ),
                  child: ColoredBox(color: config.color),
                ),
              ),
            ),
            const SizedBox(width: 14),
            const Text(
              '悬浮球 / 菜单 / 选中色',
              style: TextStyle(fontSize: 12, color: Color(0xff9aa0ab)),
            ),
          ],
        ),
        const SizedBox(height: 14),
        _slider(
          '悬浮球尺寸',
          config.ballSize.toDouble(),
          40,
          80,
          (v) =>
              _patch(config.copyWith(ballSize: v.round()), previewBall: true),
          '${config.ballSize}px',
          divisions: 20,
        ),
        const SizedBox(height: 14),
        _slider(
          '悬浮球不透明度',
          config.ballOpacity * 100,
          15,
          100,
          (v) =>
              _patch(config.copyWith(ballOpacity: v / 100), previewBall: true),
          '${(config.ballOpacity * 100).round()}%',
          divisions: 17,
        ),
        const SizedBox(height: 14),
        Row(
          children: [
            const SizedBox(
              width: 120,
              child: Text(
                '翻译目标语言',
                style: TextStyle(fontSize: 13, color: Color(0xff4e5566)),
              ),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: SizedBox(
                height: 32,
                child: TextFormField(
                  initialValue: config.targetLang,
                  textAlignVertical: TextAlignVertical.center,
                  style: const TextStyle(fontSize: 13),
                  decoration: InputDecoration(
                    isDense: true,
                    hintText: '中文',
                    constraints: const BoxConstraints.tightFor(height: 32),
                    contentPadding: const EdgeInsets.symmetric(
                      horizontal: 10,
                      vertical: 7,
                    ),
                    border: OutlineInputBorder(
                      borderRadius: BorderRadius.circular(8),
                    ),
                    enabledBorder: OutlineInputBorder(
                      borderRadius: BorderRadius.circular(8),
                      borderSide: const BorderSide(color: Color(0xffe2e4e8)),
                    ),
                  ),
                  onChanged: (value) =>
                      _patch(config.copyWith(targetLang: value)),
                ),
              ),
            ),
            const SizedBox(width: 14),
            const Text(
              '写进提示词，如 中文 / 英文 / 日文',
              style: TextStyle(fontSize: 12, color: Color(0xff9aa0ab)),
            ),
          ],
        ),
        const SizedBox(height: 14),
      ]),
      const SizedBox(height: 16),
      _section('动作', [
        const Text(
          '常用入口，和托盘菜单一致。',
          style: TextStyle(fontSize: 12, height: 1.6, color: Color(0xff8a919f)),
        ),
        const SizedBox(height: 10),
        Wrap(
          spacing: 10,
          runSpacing: 8,
          children: [
            FilledButton(
              onPressed: () => _ballAction('capture'),
              child: const Text('开始截图'),
            ),
            OutlinedButton(
              onPressed: () => _ballAction('pin-clipboard'),
              child: const Text('剪贴板贴图'),
            ),
            OutlinedButton(
              onPressed: () => _ballAction('quit'),
              child: const Text('退出'),
            ),
          ],
        ),
      ]),
      const SizedBox(height: 18),
      _saveButton(),
    ],
  );

  Widget _shortcuts(AppConfig config) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  '快捷键',
                  style: TextStyle(fontSize: 15, fontWeight: FontWeight.w600),
                ),
                SizedBox(height: 6),
                Text(
                  '点击可修改的按键绑定，直接按下新的组合键即可；Esc 放弃录制。',
                  style: TextStyle(
                    fontSize: 12,
                    height: 1.6,
                    color: Color(0xff8a919f),
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(width: 12),
          SizedBox(
            width: 212,
            height: 30,
            child: TextField(
              onChanged: (value) => setState(() => _keyQuery = value),
              style: const TextStyle(fontSize: 13),
              textAlignVertical: TextAlignVertical.center,
              decoration: InputDecoration(
                hintText: '搜索命令',
                prefixIcon: Padding(
                  padding: const EdgeInsets.only(left: 10, right: 7),
                  child: settingsIcon('search', size: 13),
                ),
                prefixIconConstraints: const BoxConstraints.tightFor(
                  width: 30,
                  height: 30,
                ),
                filled: true,
                fillColor: Color(0xfff2f3f5),
                border: OutlineInputBorder(
                  borderRadius: BorderRadius.circular(8),
                  borderSide: BorderSide.none,
                ),
                isDense: true,
                contentPadding: const EdgeInsets.symmetric(vertical: 7),
              ),
            ),
          ),
          const SizedBox(width: 8),
          OutlinedButton(
            onPressed: () => _patch(
              config.copyWith(
                shortcutCapture: Platform.isMacOS ? 'Super+1' : 'Ctrl+1',
                shortcutPinClipboard: Platform.isMacOS ? 'Super+2' : 'Ctrl+2',
              ),
            ),
            child: const Text('全部恢复默认'),
          ),
        ],
      ),
      const SizedBox(height: 16),
      Container(
        width: double.infinity,
        padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 18),
        decoration: BoxDecoration(
          border: Border.all(color: const Color(0xffeceef1)),
          borderRadius: BorderRadius.circular(12),
        ),
        child: Column(
          children: [
            const SizedBox(
              height: 35,
              child: Padding(
                padding: EdgeInsets.symmetric(horizontal: 12),
                child: Row(
                  children: [
                    Expanded(
                      flex: 65,
                      child: Text(
                        '命令',
                        style: TextStyle(
                          fontSize: 12,
                          color: Color(0xff8a919f),
                        ),
                      ),
                    ),
                    Expanded(
                      flex: 35,
                      child: Text(
                        '按键绑定',
                        style: TextStyle(
                          fontSize: 12,
                          color: Color(0xff8a919f),
                        ),
                      ),
                    ),
                    SizedBox(
                      width: 56,
                      child: Text(
                        '操作',
                        textAlign: TextAlign.right,
                        style: TextStyle(
                          fontSize: 12,
                          color: Color(0xff8a919f),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
            const Divider(height: 1, thickness: 1, color: Color(0xfff0f1f3)),
            for (final row in [
              ('开始截图', config.shortcutCapture, 'capture'),
              ('剪贴板贴图', config.shortcutPinClipboard, 'pin'),
              ('复制选区 / 确认', 'Enter', ''),
              ('取消截图 / 关闭贴图', 'Esc', ''),
              ('缩放贴图', 'Ctrl+滚轮', ''),
              ('调整贴图不透明度', 'Alt+滚轮', ''),
              ('选区取色', 'C', ''),
            ])
              if (row.$1.toLowerCase().contains(_keyQuery.trim().toLowerCase()))
                _shortcutRow(row.$1, row.$2, row.$3.isEmpty ? null : row.$3),
            if (![
              '开始截图',
              '剪贴板贴图',
              '复制选区 / 确认',
              '取消截图 / 关闭贴图',
              '缩放贴图',
              '调整贴图不透明度',
              '选区取色',
            ].any((label) => label.contains(_keyQuery.trim())))
              Padding(
                padding: const EdgeInsets.only(top: 10),
                child: Align(
                  alignment: Alignment.centerLeft,
                  child: Text(
                    '没有匹配「${_keyQuery.trim()}」的命令。',
                    style: const TextStyle(
                      fontSize: 12,
                      color: Color(0xff9aa0ab),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
      const SizedBox(height: 18),
      _saveButton(),
    ],
  );

  Widget _shortcutRow(String label, String value, String? field) => Container(
    height: field == null ? 39 : 48,
    padding: const EdgeInsets.symmetric(horizontal: 12),
    decoration: BoxDecoration(
      border: label == '选区取色'
          ? null
          : const Border(bottom: BorderSide(color: Color(0xfff5f6f7))),
    ),
    child: Row(
      children: [
        Expanded(
          flex: 65,
          child: Text(
            label,
            style: const TextStyle(fontSize: 13, color: Color(0xff1f2329)),
          ),
        ),
        Expanded(
          flex: 35,
          child: field == null
              ? Text(
                  value,
                  style: const TextStyle(
                    fontSize: 12,
                    color: Color(0xff9aa0ab),
                  ),
                )
              : Align(
                  alignment: Alignment.centerLeft,
                  child: OutlinedButton(
                    onPressed: () => _setRecording(
                      _requestedRecording == field ? null : field,
                    ),
                    style: OutlinedButton.styleFrom(
                      foregroundColor: _recording == field
                          ? _config!.color
                          : const Color(0xff4e5566),
                      backgroundColor: _recording == field
                          ? Colors.white
                          : const Color(0xfffafbfc),
                      side: BorderSide(
                        color: _recording == field
                            ? _config!.color
                            : const Color(0xffe2e4e8),
                      ),
                      minimumSize: const Size(64, 24),
                      padding: const EdgeInsets.symmetric(horizontal: 10),
                      shape: RoundedRectangleBorder(
                        borderRadius: BorderRadius.circular(6),
                      ),
                      textStyle: Theme.of(context).textTheme.bodyMedium!
                          .copyWith(fontSize: 12),
                    ),
                    child: Text(_recording == field ? '按下新组合键…' : value),
                  ),
                ),
        ),
        SizedBox(
          width: 56,
          child: field == null
              ? null
              : IconButton(
                  tooltip: '恢复默认',
                  onPressed: () {
                    final config = _config!;
                    _patch(
                      field == 'capture'
                          ? config.copyWith(
                              shortcutCapture: Platform.isMacOS
                                  ? 'Super+1'
                                  : 'Ctrl+1',
                            )
                          : config.copyWith(
                              shortcutPinClipboard: Platform.isMacOS
                                  ? 'Super+2'
                                  : 'Ctrl+2',
                            ),
                    );
                  },
                  alignment: Alignment.centerRight,
                  padding: EdgeInsets.zero,
                  icon: settingsIcon('trash', size: 15),
                ),
        ),
      ],
    ),
  );

  Widget _dataPage() => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      _heading('数据管理'),
      const Padding(
        padding: EdgeInsets.only(bottom: 16),
        child: Text(
          '配置、日志与 OCR 模型的落点。删了配置会回默认值，不影响使用。',
          style: TextStyle(fontSize: 12, height: 1.6, color: Color(0xff8a919f)),
        ),
      ),
      _section('', [
        _dataRow('配置文件', ConfigStore.file.path, 'config.json'),
        _dataRow(
          '运行日志',
          '记录应用运行与问题排查信息',
          '打开日志',
          onTap: () => _ballAction('open-log'),
        ),
        _dataRow(
          'OCR 模型',
          Platform.isMacOS
              ? 'macOS 系统本地 Vision OCR，无需下载模型文件'
              : 'Windows 系统本地 OCR，无需下载模型文件',
          '系统内置',
        ),
        _dataRow(
          '版本',
          '${Platform.isMacOS ? 'macOS' : 'Windows'} · '
              '${Abi.current().toString().split('_').last == 'x64' ? 'x86_64' : Abi.current().toString().split('_').last}',
          'v0.1.31',
          last: true,
        ),
      ]),
    ],
  );

  Widget _aboutPage() => Container(
    width: double.infinity,
    padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 4),
    decoration: BoxDecoration(
      color: Colors.white,
      borderRadius: BorderRadius.circular(14),
    ),
    child: Column(
      children: [
        SizedBox(
          height: 52,
          child: Row(
            children: [
              const Expanded(
                child: Text.rich(
                  TextSpan(
                    text: '当前版本 ',
                    children: [
                      TextSpan(
                        text: 'v0.1.31',
                        style: TextStyle(
                          fontSize: 12,
                          color: Color(0xff697586),
                        ),
                      ),
                    ],
                  ),
                  style: TextStyle(fontSize: 13),
                ),
              ),
              _aboutButton(
                '检查更新',
                () => _openUrl('https://github.com/dskcvip/SnapPin/releases'),
              ),
            ],
          ),
        ),
        SizedBox(
          height: 52,
          child: Row(
            children: [
              const Expanded(
                child: Text('关于 SnapPin', style: TextStyle(fontSize: 13)),
              ),
              _aboutButton(
                '前往官网 ↗',
                () => _openUrl('https://gitee.com/dskcvip/snappin'),
              ),
            ],
          ),
        ),
      ],
    ),
  );

  Widget _aboutButton(String label, VoidCallback onPressed) => TextButton(
    onPressed: onPressed,
    style: TextButton.styleFrom(
      foregroundColor: const Color(0xff1f2329),
      backgroundColor: const Color(0xfff0f1f3),
      minimumSize: const Size(0, 32),
      padding: const EdgeInsets.symmetric(horizontal: 12),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(8)),
    ),
    child: Text(label, style: const TextStyle(fontSize: 13)),
  );

  Widget _dataRow(
    String title,
    String detail,
    String trailing, {
    VoidCallback? onTap,
    bool last = false,
  }) => Container(
    constraints: const BoxConstraints(minHeight: 58),
    padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 13),
    decoration: BoxDecoration(
      border: last
          ? null
          : const Border(bottom: BorderSide(color: Color(0xfff5f6f7))),
    ),
    child: Row(
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: const TextStyle(fontSize: 13)),
              const SizedBox(height: 3),
              Text(
                detail,
                style: const TextStyle(fontSize: 12, color: Color(0xff9aa0ab)),
                softWrap: true,
              ),
            ],
          ),
        ),
        const SizedBox(width: 12),
        onTap == null
            ? Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
                decoration: BoxDecoration(
                  color: const Color(0xfff7f8fa),
                  borderRadius: BorderRadius.circular(6),
                ),
                child: Text(
                  trailing,
                  style: const TextStyle(
                    fontSize: 12,
                    fontFamily: 'Consolas',
                    color: Color(0xff4e5566),
                  ),
                ),
              )
            : OutlinedButton(onPressed: onTap, child: Text(trailing)),
      ],
    ),
  );

  Future<void> _ballAction(String action) async {
    final ball = await findBallWindow();
    await ball?.invokeMethod<void>('menuAction', action);
  }

  Future<void> _openUrl(String url) async {
    await Process.start(Platform.isMacOS ? 'open' : 'explorer.exe', [url]);
  }

  Widget _heading(String title) => Padding(
    padding: const EdgeInsets.only(bottom: 6),
    child: Text(
      title,
      style: const TextStyle(
        fontSize: 15,
        height: 1.2,
        fontWeight: FontWeight.w600,
      ),
    ),
  );

  Widget _navItem(String label, IconData icon) => SizedBox(
    height: 34,
    child: Material(
      color: _page == label ? const Color(0xffe6e8ec) : Colors.transparent,
      borderRadius: BorderRadius.circular(8),
      child: InkWell(
        borderRadius: BorderRadius.circular(8),
        onTap: () {
          _setRecording(null);
          setState(() => _page = label);
        },
        child: Row(
          children: [
            const SizedBox(width: 10),
            settingsIcon(
              switch (label) {
                '通用' => 'general',
                '快捷键' => 'keys',
                '模型' => 'model',
                '数据管理' => 'data',
                _ => 'about',
              },
              color: _page == label
                  ? const Color(0xff4e5566)
                  : const Color(0xff8a919f),
            ),
            const SizedBox(width: 9),
            Text(
              label,
              style: TextStyle(
                fontSize: 13,
                fontWeight: _page == label ? FontWeight.w500 : FontWeight.w400,
                color: _page == label
                    ? const Color(0xff1f2329)
                    : const Color(0xff4e5566),
              ),
            ),
          ],
        ),
      ),
    ),
  );

  Widget _section(String title, List<Widget> children) => Container(
    width: double.infinity,
    padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 18),
    decoration: BoxDecoration(
      border: Border.all(color: const Color(0xffeceef1)),
      borderRadius: BorderRadius.circular(12),
    ),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (title.isNotEmpty) ...[
          Text(
            title,
            style: const TextStyle(
              fontSize: 15,
              height: 1.2,
              fontWeight: FontWeight.w600,
            ),
          ),
          const SizedBox(height: 6),
        ],
        ...children,
      ],
    ),
  );

  Widget _slider(
    String label,
    double value,
    double min,
    double max,
    ValueChanged<double> change,
    String display, {
    int? divisions,
  }) => SizedBox(
    height: 20,
    child: Row(
      children: [
        SizedBox(
          width: 120,
          child: Text(
            label,
            style: const TextStyle(fontSize: 13, color: Color(0xff4e5566)),
          ),
        ),
        const SizedBox(width: 14),
        Expanded(
          child: SliderTheme(
            data: SliderTheme.of(context).copyWith(
              trackHeight: 6,
              activeTrackColor: const Color(0xff3478ff),
              inactiveTrackColor: const Color(0xffefefef),
              thumbColor: const Color(0xff3478ff),
              thumbShape: const RoundSliderThumbShape(
                enabledThumbRadius: 7.5,
                elevation: 0,
                pressedElevation: 0,
              ),
              overlayShape: SliderComponentShape.noOverlay,
              tickMarkShape: SliderTickMarkShape.noTickMark,
              trackShape: const _NativeRangeTrack(),
            ),
            child: Slider(
              padding: const EdgeInsets.symmetric(horizontal: 2),
              value: value,
              min: min,
              max: max,
              divisions: divisions,
              onChanged: change,
            ),
          ),
        ),
        const SizedBox(width: 14),
        Text(
          display,
          style: const TextStyle(fontSize: 12, color: Color(0xff9aa0ab)),
          textAlign: TextAlign.right,
        ),
      ],
    ),
  );
}

class _NativeRangeTrack extends RoundedRectSliderTrackShape {
  const _NativeRangeTrack();
  @override
  void paint(
    PaintingContext context,
    Offset offset, {
    required RenderBox parentBox,
    required SliderThemeData sliderTheme,
    required Animation<double> enableAnimation,
    required Offset thumbCenter,
    Offset? secondaryOffset,
    bool isDiscrete = false,
    bool isEnabled = false,
    required TextDirection textDirection,
    double additionalActiveTrackHeight = 0,
  }) {
    final rect = getPreferredRect(
      parentBox: parentBox,
      offset: offset,
      sliderTheme: sliderTheme,
      isEnabled: isEnabled,
      isDiscrete: isDiscrete,
    );
    final rounded = RRect.fromRectAndRadius(rect, const Radius.circular(3));
    final canvas = context.canvas;
    canvas.drawRRect(rounded, Paint()..color = const Color(0xffefefef));
    canvas.drawRRect(
      rounded,
      Paint()
        ..color = const Color(0xffaaaaaa)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
    canvas.save();
    canvas.clipRRect(rounded);
    canvas.drawRect(
      Rect.fromLTRB(rect.left, rect.top, thumbCenter.dx, rect.bottom),
      Paint()..color = const Color(0xff3478ff),
    );
    canvas.restore();
  }
}
