import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:pasteboard/pasteboard.dart';
import 'package:screen_retriever/screen_retriever.dart';
import 'package:window_manager/window_manager.dart';

import '../pin_edits.dart';
import '../annotation.dart';
import '../capture_image.dart';
import '../image_save.dart';
import '../app_log.dart';
import 'image_pixels.dart';

class PinWindow extends StatefulWidget {
  const PinWindow({
    super.key,
    required this.controller,
    required this.path,
    required this.baseSize,
  });
  final WindowController controller;
  final String path;
  final Size baseSize;

  @override
  State<PinWindow> createState() => _PinWindowState();
}

class _PinWindowState extends State<PinWindow> with WindowListener {
  double _opacity = 1;
  double _scale = 1;
  bool _topmost = true;
  bool _locked = false;
  bool _shadow = true;
  bool _toolbar = true;
  bool _passThrough = false;
  Timer? _passTimer;
  String _group = '默认';
  late Size _baseSize;
  late Size _editSourceSize;
  late Future<Uint8List> _sourceBytes;
  String _displayPath = '';
  PinEdits _edits = const PinEdits();
  bool _editing = false;
  bool _fit = false;
  bool _viewerEditing = false;
  bool _fullscreen = false;
  bool _maximized = false;
  bool _favorite = false;
  AnnotationTool? _editTool;
  bool _cropMode = false;
  final List<Annotation> _marks = [];
  final List<Annotation> _undoneMarks = [];
  Annotation? _draftMark;
  Rect? _cropRect;
  Offset? _cropStart;
  final TextEditingController _textInput = TextEditingController();

  @override
  void initState() {
    super.initState();
    windowManager.addListener(this);
    _baseSize = widget.baseSize;
    _editSourceSize = widget.baseSize;
    _displayPath = widget.path;
    _sourceBytes = File(widget.path).readAsBytes();
    widget.controller.setWindowMethodHandler((call) async {
      switch (call.method) {
        case 'getPinInfo':
          return jsonEncode({
            'id': widget.controller.windowId,
            'path': _displayPath,
            'width': _baseSize.width * _scale,
            'height': _baseSize.height * _scale,
            'group': _group,
          });
        case 'getPinFlags':
          return jsonEncode({
            'opacity': _opacity,
            'topmost': _topmost,
            'locked': _locked,
            'shadow': _shadow,
            'toolbar': _toolbar,
            'rotate': _edits.quarterTurns,
            'flipH': _edits.flipHorizontal,
            'flipV': _edits.flipVertical,
            'gray': _edits.grayscale,
            'invert': _edits.invert,
            'alpha': _edits.alpha,
            'brightness': _edits.brightness,
            'group': _group,
          });
        case 'focusPin':
          await windowManager.show();
          await windowManager.focus();
        case 'closePin':
          unawaited(Future<void>.delayed(Duration.zero, windowManager.close));
        case 'setPinGroup':
          if (mounted) {
            final group = (call.arguments as String).trim();
            setState(() => _group = group.isEmpty ? '默认' : group);
          }
        case 'hidePin':
          await windowManager.hide();
        case 'showPin':
          await windowManager.show();
        case 'setPinOpacity':
          if (mounted) {
            setState(
              () => _opacity = (call.arguments as num).toDouble().clamp(.15, 1),
            );
          }
        case 'pinAction':
          await _performAction(call.arguments as String);
      }
      return null;
    });
  }

  @override
  void dispose() {
    windowManager.removeListener(this);
    _passTimer?.cancel();
    _textInput.dispose();
    if (_displayPath != widget.path) {
      final path = _displayPath;
      unawaited(File(path).delete().catchError((_) => File(path)));
    }
    super.dispose();
  }

  @override
  void onWindowMaximize() {
    if (mounted) setState(() => _maximized = true);
  }

  @override
  void onWindowUnmaximize() {
    if (mounted) setState(() => _maximized = false);
  }

  @override
  void onWindowResize() {
    _syncMaximized();
  }

  Future<void> _syncMaximized() async {
    final maximized = await windowManager.isMaximized();
    if (mounted && _maximized != maximized) {
      setState(() => _maximized = maximized);
    }
  }

  Future<void> _toggleMaximized() async {
    final maximized = await windowManager.isMaximized();
    if (maximized) {
      await windowManager.unmaximize();
    } else {
      await windowManager.maximize();
    }
    if (mounted) setState(() => _maximized = !maximized);
    await _syncMaximized();
    AppLog.write('pin-viewer-maximize', 'maximized=${!maximized}');
  }

  Future<void> _performAction(String action) async {
    AppLog.write(
      'pin-action',
      'window=${widget.controller.windowId} action=$action',
    );
    switch (action) {
      case 'copy':
        await _copy();
      case 'save':
        await _save();
      case 'close':
        unawaited(Future<void>.delayed(Duration.zero, windowManager.close));
      case 'topmost':
        await _toggleTopmost();
      case 'lock':
        if (mounted) setState(() => _locked = !_locked);
      case 'shadow':
        if (mounted) setState(() => _shadow = !_shadow);
      case 'toolbar':
        if (mounted) setState(() => _toolbar = !_toolbar);
      case 'pass-through':
        await _enablePassThrough();
      default:
        await _applyEdit(action);
    }
  }

  Future<void> _applyEdit(String action) async {
    if (_editing) return;
    var next = _edits;
    switch (action) {
      case 'rotate-right':
        next = next.copyWith(quarterTurns: next.quarterTurns + 1);
      case 'rotate-left':
        next = next.copyWith(quarterTurns: next.quarterTurns + 3);
      case 'flip-h':
        next = next.copyWith(flipHorizontal: !next.flipHorizontal);
      case 'flip-v':
        next = next.copyWith(flipVertical: !next.flipVertical);
      case 'grayscale':
        next = next.copyWith(grayscale: !next.grayscale);
      case 'invert':
        next = next.copyWith(invert: !next.invert);
      case 'alpha-100':
        next = next.copyWith(alpha: 1);
      case 'alpha-75':
        next = next.copyWith(alpha: .75);
      case 'alpha-50':
        next = next.copyWith(alpha: .5);
      case 'alpha-25':
        next = next.copyWith(alpha: .25);
      case 'bright-up':
        next = next.copyWith(brightness: (next.brightness + .1).clamp(.1, 2));
      case 'bright-down':
        next = next.copyWith(brightness: (next.brightness - .1).clamp(.1, 2));
      case 'bright-reset':
        next = next.copyWith(brightness: 1);
      case 'reset-edit':
        next = const PinEdits();
      default:
        return;
    }
    _editing = true;
    try {
      final bytes = await renderPinEdits(await _sourceBytes, next);
      final path =
          '${File(widget.path).parent.path}${Platform.pathSeparator}'
          'edit-${DateTime.now().microsecondsSinceEpoch}.png';
      await File(path).writeAsBytes(bytes);
      final oldPath = _displayPath;
      _edits = next;
      _baseSize = next.quarterTurns.isOdd
          ? Size(_editSourceSize.height, _editSourceSize.width)
          : _editSourceSize;
      if (mounted) setState(() => _displayPath = path);
      await _resizeForEdit();
      if (oldPath != widget.path) {
        unawaited(File(oldPath).delete().catchError((_) => File(oldPath)));
      }
    } finally {
      _editing = false;
    }
  }

  Future<void> _resizeForEdit() async {
    AppLog.write('pin-image-size', '${_baseSize.width}x${_baseSize.height}');
  }

  Future<void> _resizeForScale(double scale) async {
    if (scale == _scale) return;
    setState(() => _scale = scale);
    await _resizeForEdit();
    AppLog.write(
      'pin-scale',
      'window=${widget.controller.windowId} scale=$scale',
    );
  }

  Future<void> _completeViewerEdit() async {
    if (_marks.isEmpty && _cropRect == null) {
      setState(() => _viewerEditing = false);
      return;
    }
    if (_editing) return;
    _editing = true;
    try {
      final codec = await ui.instantiateImageCodec(
        await File(_displayPath).readAsBytes(),
      );
      final frame = await codec.getNextFrame();
      final source = frame.image;
      final crop =
          _cropRect ?? Rect.fromLTWH(0, 0, _baseSize.width, _baseSize.height);
      final bytes = await cropSelection(source, crop, _baseSize, _marks);
      source.dispose();
      codec.dispose();
      final path =
          '${File(widget.path).parent.path}${Platform.pathSeparator}'
          'viewer-edit-${DateTime.now().microsecondsSinceEpoch}.png';
      await File(path).writeAsBytes(bytes);
      final oldPath = _displayPath;
      setState(() {
        _displayPath = path;
        _baseSize = crop.size;
        _editSourceSize = crop.size;
        _sourceBytes = Future.value(bytes);
        _edits = const PinEdits();
        _viewerEditing = false;
        _marks.clear();
        _cropRect = null;
        _editTool = null;
        _cropMode = false;
      });
      if (oldPath != widget.path) {
        unawaited(File(oldPath).delete().catchError((_) => File(oldPath)));
      }
      AppLog.write(
        'pin-viewer-edit-saved',
        '${_baseSize.width}x${_baseSize.height}',
      );
    } catch (error, stack) {
      AppLog.write('pin-viewer-edit-failed', '$error\n$stack');
      if (mounted) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('编辑失败：$error')));
      }
    } finally {
      _editing = false;
    }
  }

  Future<void> _toggleTopmost() async {
    final next = !_topmost;
    await windowManager.setAlwaysOnTop(next);
    if (mounted) setState(() => _topmost = next);
  }

  Future<void> _enablePassThrough() async {
    _passTimer?.cancel();
    setState(() => _passThrough = true);
    await windowManager.setIgnoreMouseEvents(true);
    _passTimer = Timer(const Duration(seconds: 5), () async {
      await windowManager.setIgnoreMouseEvents(false);
      if (mounted) setState(() => _passThrough = false);
    });
  }

  Future<void> _openMenu(Offset localPosition) async {
    final position = await windowManager.getPosition();
    final point = position + localPosition;
    final displays = await screenRetriever.getAllDisplays();
    final display =
        displays.where((item) {
          final origin = item.visiblePosition ?? Offset.zero;
          final size = item.visibleSize ?? item.size;
          return (origin & size).contains(point);
        }).firstOrNull ??
        await screenRetriever.getPrimaryDisplay();
    final visible = display.visiblePosition ?? Offset.zero;
    final size = display.visibleSize ?? display.size;
    final x = point.dx.clamp(
      visible.dx,
      (visible.dx + size.width - 650).clamp(visible.dx, double.infinity),
    );
    final y = point.dy.clamp(
      visible.dy,
      (visible.dy + size.height - 620).clamp(visible.dy, double.infinity),
    );
    await WindowController.create(
      WindowConfiguration(
        arguments: jsonEncode({
          'role': 'pin-menu',
          'target': widget.controller.windowId,
          'x': x,
          'y': y,
        }),
        hiddenAtLaunch: true,
      ),
    );
  }

  Future<void> _copy() async =>
      Pasteboard.writeImage(await File(_displayPath).readAsBytes());

  Future<void> _save() async {
    await saveImage(await File(_displayPath).readAsBytes());
  }

  void _startMark(Offset point) {
    if (!_viewerEditing) return;
    if (_cropMode) {
      setState(() {
        _cropStart = point;
        _cropRect = Rect.fromPoints(point, point);
      });
      return;
    }
    final tool = _editTool;
    if (tool == null) return;
    setState(
      () => _draftMark = Annotation(
        tool: tool,
        points: [point],
        color: const Color(0xffed3030),
        strokeWidth: tool == AnnotationTool.sticker ? 36 : 3,
        text: tool == AnnotationTool.sticker ? '😊' : null,
        parameter: tool == AnnotationTool.mosaic ? 12 : 0,
      ),
    );
  }

  void _updateMark(Offset point) {
    if (_cropStart != null) {
      setState(() => _cropRect = Rect.fromPoints(_cropStart!, point));
      return;
    }
    final draft = _draftMark;
    if (draft == null) return;
    setState(
      () => _draftMark = Annotation(
        tool: draft.tool,
        points: draft.tool == AnnotationTool.pen
            ? [...draft.points, point]
            : [draft.points.first, point],
        color: draft.color,
        strokeWidth: draft.strokeWidth,
        text: draft.text,
        parameter: draft.parameter,
      ),
    );
  }

  void _endMark() {
    if (_cropStart != null) {
      setState(() => _cropStart = null);
      return;
    }
    final draft = _draftMark;
    if (draft == null) return;
    setState(() {
      _marks.add(draft);
      _undoneMarks.clear();
      _draftMark = null;
    });
  }

  Future<void> _insertText(Offset point) async {
    _textInput.clear();
    final value = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('添加文字'),
        content: TextField(controller: _textInput, autofocus: true),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('取消'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, _textInput.text),
            child: const Text('确定'),
          ),
        ],
      ),
    );
    if (value == null || value.trim().isEmpty || !mounted) return;
    setState(
      () => _marks.add(
        Annotation(
          tool: AnnotationTool.text,
          points: [point],
          color: const Color(0xffed3030),
          strokeWidth: 24,
          text: value.trim(),
        ),
      ),
    );
  }

  Future<void> _recognizePinText() async {
    try {
      final ball = await WindowController.getAll();
      final target = ball
          .where((window) => window.arguments.isEmpty)
          .firstOrNull;
      if (target == null) throw StateError('主窗口不可用');
      final text = await target.invokeMethod<String>(
        'recognizeText',
        await File(_displayPath).readAsBytes(),
      );
      await WindowController.create(
        WindowConfiguration(
          arguments: jsonEncode({
            'role': 'ocr',
            'image': _displayPath,
            'text': text ?? '',
          }),
          hiddenAtLaunch: true,
        ),
      );
    } catch (error) {
      AppLog.write('pin-ocr-failed', error);
      if (mounted)
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('文字识别失败：$error')));
    }
  }

  Future<void> _openWithDefault() async {
    await Process.start(Platform.isMacOS ? 'open' : 'explorer.exe', [
      _displayPath,
    ]);
  }

  Future<void> _printImage() async {
    if (Platform.isMacOS) {
      final result = await Process.run('lp', [_displayPath]);
      if (result.exitCode != 0) throw StateError('${result.stderr}');
      return;
    }
    final quoted = _displayPath.replaceAll("'", "''");
    final result = await Process.run('powershell.exe', [
      '-NoProfile',
      '-Command',
      "Start-Process -LiteralPath '$quoted' -Verb Print",
    ]);
    if (result.exitCode != 0) throw StateError('${result.stderr}');
  }

  Future<void> _moreAction(String action) async {
    try {
      switch (action) {
        case 'copy':
          await _copy();
        case 'print':
          await _printImage();
        case 'favorite':
          setState(() => _favorite = !_favorite);
        case 'open':
          await _openWithDefault();
      }
      AppLog.write('pin-viewer-more', action);
    } catch (error) {
      AppLog.write('pin-viewer-more-failed', '$action $error');
      if (mounted)
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('操作失败：$error')));
    }
  }

  Widget _viewerIcon(
    IconData icon,
    String label,
    VoidCallback? action, {
    bool active = false,
  }) => Tooltip(
    message: label,
    child: SizedBox(
      width: 40,
      height: 40,
      child: Center(
        child: Material(
          color: active ? const Color(0xffd5d5d7) : Colors.transparent,
          borderRadius: BorderRadius.circular(3),
          child: InkWell(
            onTap: action,
            borderRadius: BorderRadius.circular(3),
            child: SizedBox(
              width: 28,
              height: 28,
              child: Icon(
                icon,
                size: 18,
                color: action == null
                    ? const Color(0xffb5b5b5)
                    : const Color(0xff57585a),
              ),
            ),
          ),
        ),
      ),
    ),
  );

  Widget _divider() =>
      Container(width: 1, height: 20, color: const Color(0xffcfcfd1));

  Widget _viewerToolbar() => Container(
    height: 41,
    color: const Color(0xffe8e8ea),
    child: Row(
      children: [
        _viewerIcon(
          Icons.push_pin_outlined,
          '窗口置顶',
          _toggleTopmost,
          active: _topmost,
        ),
        _divider(),
        if (!_viewerEditing) ...[
          _viewerIcon(Icons.arrow_back, '上一张', null),
          _viewerIcon(Icons.arrow_forward, '下一张', null),
          _divider(),
          _viewerIcon(Icons.zoom_in, '放大', () {
            _fit = false;
            _resizeForScale((_scale * 1.25).clamp(.05, 8));
          }),
          _viewerIcon(Icons.zoom_out, '缩小', () {
            _fit = false;
            _resizeForScale((_scale / 1.25).clamp(.05, 8));
          }),
          _viewerIcon(
            _fit ? Icons.aspect_ratio : Icons.fit_screen,
            _fit ? '图片原始大小' : '图片适应窗口大小',
            () => setState(() {
              _fit = !_fit;
              _scale = 1;
            }),
            active: _fit,
          ),
          _divider(),
          _viewerIcon(
            Icons.rotate_right,
            '向右旋转',
            () => _applyEdit('rotate-right'),
          ),
          _viewerIcon(
            Icons.edit_outlined,
            '编辑',
            () => setState(() => _viewerEditing = true),
          ),
          _divider(),
          _viewerIcon(
            Icons.document_scanner_outlined,
            '文字识别',
            _recognizePinText,
          ),
          _viewerIcon(Icons.fullscreen, '全屏', () async {
            _fullscreen = !_fullscreen;
            await windowManager.setFullScreen(_fullscreen);
            if (mounted) setState(() {});
          }),
          _viewerIcon(Icons.file_download_outlined, '保存', _save),
        ] else ...[
          _viewerIcon(
            Icons.crop_square,
            '矩形',
            () => setState(() {
              _cropMode = false;
              _editTool = AnnotationTool.rectangle;
            }),
            active: _editTool == AnnotationTool.rectangle,
          ),
          _viewerIcon(
            Icons.circle_outlined,
            '椭圆',
            () => setState(() {
              _cropMode = false;
              _editTool = AnnotationTool.ellipse;
            }),
            active: _editTool == AnnotationTool.ellipse,
          ),
          _viewerIcon(
            Icons.sentiment_satisfied_alt_outlined,
            '贴纸',
            () => setState(() {
              _cropMode = false;
              _editTool = AnnotationTool.sticker;
            }),
            active: _editTool == AnnotationTool.sticker,
          ),
          _viewerIcon(
            Icons.north_east,
            '箭头',
            () => setState(() {
              _cropMode = false;
              _editTool = AnnotationTool.arrow;
            }),
            active: _editTool == AnnotationTool.arrow,
          ),
          _viewerIcon(
            Icons.edit,
            '画笔',
            () => setState(() {
              _cropMode = false;
              _editTool = AnnotationTool.pen;
            }),
            active: _editTool == AnnotationTool.pen,
          ),
          _viewerIcon(
            Icons.blur_on,
            '马赛克',
            () => setState(() {
              _cropMode = false;
              _editTool = AnnotationTool.mosaic;
            }),
            active: _editTool == AnnotationTool.mosaic,
          ),
          _viewerIcon(
            Icons.text_fields,
            '文字',
            () => setState(() {
              _cropMode = false;
              _editTool = AnnotationTool.text;
            }),
            active: _editTool == AnnotationTool.text,
          ),
          _viewerIcon(
            Icons.crop,
            '裁剪',
            () => setState(() {
              _cropMode = true;
              _editTool = null;
            }),
            active: _cropMode,
          ),
          _divider(),
          _viewerIcon(
            Icons.undo,
            '撤销',
            _marks.isEmpty
                ? null
                : () => setState(() {
                    _undoneMarks.add(_marks.removeLast());
                  }),
          ),
          _viewerIcon(Icons.file_download_outlined, '保存', () async {
            await _completeViewerEdit();
            if (!_viewerEditing) await _save();
          }),
          _viewerIcon(
            Icons.redo,
            '恢复',
            _undoneMarks.isEmpty
                ? null
                : () => setState(() {
                    _marks.add(_undoneMarks.removeLast());
                  }),
          ),
        ],
        Expanded(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onPanStart: _locked ? null : (_) => windowManager.startDragging(),
          ),
        ),
        if (_viewerEditing)
          Padding(
            padding: const EdgeInsets.only(right: 10),
            child: TextButton(
              onPressed: _completeViewerEdit,
              style: TextButton.styleFrom(
                backgroundColor: const Color(0xff00c78d),
                foregroundColor: Colors.white,
                minimumSize: const Size(52, 26),
              ),
              child: const Text('完成', style: TextStyle(fontSize: 12)),
            ),
          )
        else
          PopupMenuButton<String>(
            tooltip: '更多',
            onSelected: _moreAction,
            constraints: const BoxConstraints.tightFor(width: 160),
            itemBuilder: (_) => [
              const PopupMenuItem(
                value: 'copy',
                height: 36,
                padding: EdgeInsets.symmetric(horizontal: 10),
                child: Text('复制', style: TextStyle(fontSize: 14)),
              ),
              const PopupMenuItem(
                value: 'print',
                height: 36,
                padding: EdgeInsets.symmetric(horizontal: 10),
                child: Text('打印', style: TextStyle(fontSize: 14)),
              ),
              PopupMenuItem(
                value: 'favorite',
                height: 36,
                padding: const EdgeInsets.symmetric(horizontal: 10),
                child: Text(
                  _favorite ? '取消收藏' : '收藏',
                  style: const TextStyle(fontSize: 14),
                ),
              ),
              const PopupMenuItem(
                value: 'open',
                height: 36,
                padding: EdgeInsets.symmetric(horizontal: 10),
                child: Text('使用系统默认方式打开', style: TextStyle(fontSize: 14)),
              ),
            ],
            child: const SizedBox(
              width: 40,
              height: 40,
              child: Icon(Icons.more_horiz, size: 18, color: Color(0xff57585a)),
            ),
          ),
        _divider(),
        _viewerIcon(Icons.minimize, '最小化', windowManager.minimize),
        _viewerIcon(
          _maximized ? Icons.filter_none : Icons.crop_square,
          _maximized ? '还原' : '最大化',
          _toggleMaximized,
        ),
        _viewerIcon(Icons.close, '关闭', windowManager.close),
      ],
    ),
  );

  Widget _viewerImage(BoxConstraints bounds) {
    final fitScale = math.min(
      bounds.maxWidth / _baseSize.width,
      bounds.maxHeight / _baseSize.height,
    );
    final scale = (_fit ? fitScale : _scale).clamp(.01, 8.0);
    final imageWidth = _baseSize.width * scale;
    final imageHeight = _baseSize.height * scale;
    Offset sourcePoint(Offset local) => Offset(
      (local.dx / scale).clamp(0, _baseSize.width),
      (local.dy / scale).clamp(0, _baseSize.height),
    );
    final image = GestureDetector(
      onDoubleTap: _viewerEditing
          ? null
          : () => setState(() {
              _fit = !_fit;
              _scale = 1;
            }),
      onTapUp: _viewerEditing && _editTool == AnnotationTool.text
          ? (details) => _insertText(sourcePoint(details.localPosition))
          : null,
      onPanStart: _viewerEditing && _editTool != AnnotationTool.text
          ? (details) => _startMark(sourcePoint(details.localPosition))
          : null,
      onPanUpdate: _viewerEditing && _editTool != AnnotationTool.text
          ? (details) => _updateMark(sourcePoint(details.localPosition))
          : null,
      onPanEnd: _viewerEditing && _editTool != AnnotationTool.text
          ? (_) => _endMark()
          : null,
      child: SizedBox(
        width: imageWidth,
        height: imageHeight,
        child: Stack(
          children: [
            Positioned.fill(
              child: ImagePixels(
                path: _displayPath,
                sourceHeight: _baseSize.height.round(),
                displayWidth: imageWidth,
                scale: scale,
                opacity: _opacity,
              ),
            ),
            if (_marks.isNotEmpty || _draftMark != null || _cropRect != null)
              Positioned.fill(
                child: IgnorePointer(
                  child: CustomPaint(
                    painter: _PinMarksPainter(
                      marks: _marks,
                      draft: _draftMark,
                      crop: _cropRect,
                      sourceSize: _baseSize,
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
    return Container(
      color: const Color(0xfff8f8f8),
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: SizedBox(
          width: math.max(bounds.maxWidth, imageWidth),
          child: SingleChildScrollView(
            scrollDirection: Axis.vertical,
            child: SizedBox(
              height: math.max(bounds.maxHeight, imageHeight),
              child: Center(child: image),
            ),
          ),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) => Focus(
    autofocus: true,
    onKeyEvent: (_, event) {
      if (event is! KeyDownEvent) return KeyEventResult.ignored;
      if (event.logicalKey == LogicalKeyboardKey.escape) {
        if (_viewerEditing)
          setState(() => _viewerEditing = false);
        else
          windowManager.close();
        return KeyEventResult.handled;
      }
      if (event.logicalKey == LogicalKeyboardKey.keyT) {
        _toggleTopmost();
        return KeyEventResult.handled;
      }
      if (event.logicalKey == LogicalKeyboardKey.keyH) {
        setState(() => _toolbar = !_toolbar);
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    },
    child: Listener(
      onPointerDown: (event) {
        if (event.buttons == 2) _openMenu(event.localPosition);
      },
      onPointerSignal: (event) {
        if (event is! PointerScrollEvent) return;
        final keyboard = HardwareKeyboard.instance;
        if (keyboard.isControlPressed) {
          _fit = false;
          _resizeForScale(
            (_scale * (event.scrollDelta.dy < 0 ? 1.1 : .9)).clamp(.05, 8),
          );
        } else if (keyboard.isAltPressed) {
          setState(
            () =>
                _opacity = (_opacity + (event.scrollDelta.dy < 0 ? .05 : -.05))
                    .clamp(.15, 1),
          );
        }
      },
      child: Stack(
        children: [
          Column(
            children: [
              if (_toolbar) _viewerToolbar(),
              Expanded(
                child: LayoutBuilder(
                  builder: (_, bounds) => _viewerImage(bounds),
                ),
              ),
            ],
          ),
          if (_passThrough)
            const Positioned(
              bottom: 16,
              left: 0,
              right: 0,
              child: Center(
                child: DecoratedBox(
                  decoration: BoxDecoration(color: Color(0xff4b4b4b)),
                  child: Padding(
                    padding: EdgeInsets.symmetric(horizontal: 12, vertical: 7),
                    child: Text(
                      '鼠标穿透中 · 5 秒后自动恢复',
                      style: TextStyle(color: Colors.white, fontSize: 12),
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

class _PinMarksPainter extends CustomPainter {
  const _PinMarksPainter({
    required this.marks,
    required this.draft,
    required this.crop,
    required this.sourceSize,
  });

  final List<Annotation> marks;
  final Annotation? draft;
  final Rect? crop;
  final Size sourceSize;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.save();
    canvas.scale(
      size.width / sourceSize.width,
      size.height / sourceSize.height,
    );
    for (final mark in marks) {
      mark.paint(canvas);
    }
    draft?.paint(canvas);
    if (crop != null) {
      canvas.drawRect(
        crop!,
        Paint()
          ..color = const Color(0xff167cde)
          ..strokeWidth = 2
          ..style = PaintingStyle.stroke,
      );
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant _PinMarksPainter oldDelegate) => true;
}
