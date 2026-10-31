import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:pasteboard/pasteboard.dart';
import 'package:screen_retriever/screen_retriever.dart';
import 'package:window_manager/window_manager.dart';

import '../capture_bridge.dart';
import 'capture_exit_dialog.dart';
import 'ocr_window.dart' show showOcrResult;
import '../app_log.dart';
import '../image_save.dart';
import '../capture_image.dart';
import '../annotation.dart';
import '../qr_scan.dart';
import '../shot_icons.dart';
import '../window_roles.dart';

class CaptureOverlay extends StatefulWidget {
  const CaptureOverlay({
    super.key,
    this.display,
    required this.controller,
    this.prewarmBounds,
  });
  final CapturedDisplay? display;
  final WindowController controller;
  final Rect? prewarmBounds;

  @override
  State<CaptureOverlay> createState() => _CaptureOverlayState();
}

class _CaptureOverlayState extends State<CaptureOverlay> {
  CapturedDisplay? _currentDisplay;
  Rect? _windowBounds;
  CapturedDisplay get _display => _currentDisplay!;
  Offset? _start;
  Offset? _cursor;
  Rect? _selection;
  Rect? _hoverWindow;
  Rect? _pressedWindow;
  int _snapSequence = 0;
  DateTime? _lastSnap;
  Rect? _resizeOrigin;
  Offset? _resizePointer;
  String? _resizeEdge;
  ui.Image? _image;
  ByteData? _pixels;
  bool _closing = false;
  DateTime? _lastSelectionClick;
  Offset? _lastSelectionClickPoint;
  bool _hoverLogged = false;
  AnnotationTool? _tool;
  final List<Annotation> _annotations = [];
  final List<Annotation> _redo = [];
  Annotation? _draft;
  Annotation? _editingText;
  TextEditingController? _textController;
  FocusNode? _textFocus;
  Color _color = const Color(0xffe62d2d);
  double _strokeWidth = 2;
  double _fontSize = 16;
  double _mosaicBlock = 12;
  double _spotDim = 150;
  double _eraserRadius = 16;
  double _stickerSize = 48;
  String _annotationText = '文字';
  String _sticker = '✅';
  String? _stickerIconName;
  ui.Picture? _stickerPicture;
  Size? _stickerPictureSize;
  final List<ui.Picture> _stickerPictures = [];
  Map<String, String>? _stickerIcons;
  Map<String, String> _stickerZh = const {};
  Map<String, List<String>> _stickerSynonyms = const {};
  Color _stickerColor = const Color(0xff111827);
  bool _stickerColorOpen = false;
  bool _stickerSizeOpen = false;
  OverlayEntry? _stickerFlyout;
  final Object _stickerFlyoutGroup = Object();
  int _badgeNumber = 1;
  bool _qrMode = false;
  Offset? _qrStart;
  Rect? _qrRect;
  List<DecodedCode>? _qrCodes;
  bool _qrBusy = false;
  String? _ocrText;
  bool _ocrBusy = false;
  bool _transBusy = false;
  bool _longMode = false;
  bool _confirmingExit = false;
  DialogRoute<CaptureExitDecision>? _exitRoute;

  @override
  void initState() {
    super.initState();
    _currentDisplay = widget.display;
    _windowBounds = widget.prewarmBounds;
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'prewarmReady') return true;
      if (call.method == 'closeSelf') {
        unawaited(Future<void>.delayed(Duration.zero, _closeSelf));
      } else if (call.method == 'startCapture') {
        final values = Map<String, dynamic>.from(call.arguments as Map);
        final display = CapturedDisplay.fromMap(values);
        final cursor = values['cursorX'] is num && values['cursorY'] is num
            ? Offset(
                (values['cursorX'] as num).toDouble(),
                (values['cursorY'] as num).toDouble(),
              )
            : null;
        await _startCapture(display, screenCursor: cursor);
      } else if (call.method == 'clearSelection') {
        if (mounted)
          setState(() {
            _selection = null;
            _hoverWindow = null;
            _annotations.clear();
            _redo.clear();
            _tool = null;
          });
      } else if (call.method == 'clearHover') {
        if (mounted) setState(() => _hoverWindow = null);
      } else if (call.method == 'longError') {
        await windowManager.setIgnoreMouseEvents(false);
        if (mounted)
          setState(() {
            _longMode = false;
            _ocrText = call.arguments as String;
          });
      }
      return null;
    });
    if (_currentDisplay != null) _loadImage();
  }

  Future<void> _startCapture(
    CapturedDisplay display, {
    Offset? screenCursor,
  }) async {
    _finishTextEditor(commit: false);
    _hideStickerFlyout();
    _image?.dispose();
    _image = null;
    _pixels = null;
    _closing = false;
    _hoverLogged = false;
    _snapSequence++;
    _lastSnap = null;
    setState(() {
      _currentDisplay = display;
      _longMode = false;
      _selection = null;
      _hoverWindow = null;
      _pressedWindow = null;
      _cursor = null;
      _annotations.clear();
      _redo.clear();
      _draft = null;
      _tool = null;
      _qrMode = false;
      _qrRect = null;
      _qrCodes = null;
      _ocrText = null;
    });
    final bounds = Rect.fromLTWH(
      display.x,
      display.y,
      display.width,
      display.height,
    );
    if (_windowBounds?.size != bounds.size) {
      await windowManager.setSize(bounds.size);
    }
    if (_windowBounds?.topLeft != bounds.topLeft) {
      await windowManager.setPosition(bounds.topLeft);
    }
    _windowBounds = bounds;
    final cursor = screenCursor ?? await screenRetriever.getCursorScreenPoint();
    final localCursor = cursor - Offset(display.x, display.y);
    if ((Offset.zero & Size(display.width, display.height)).contains(
      localCursor,
    )) {
      setState(() => _cursor = localCursor);
      AppLog.write(
        'overlay-initial-cursor',
        '${localCursor.dx.round()},${localCursor.dy.round()}',
      );
    }
    unawaited(_loadImage());
    await windowManager.show();
    await windowManager.focus();
  }

  Future<void> _loadImage() async {
    final display = _currentDisplay;
    if (display == null) return;
    final bytes = await File(display.path).readAsBytes();
    final codec = await ui.instantiateImageCodec(bytes);
    final frame = await codec.getNextFrame();
    final pixels = await frame.image.toByteData(
      format: ui.ImageByteFormat.rawRgba,
    );
    if (!mounted || _currentDisplay != display) {
      frame.image.dispose();
      return;
    }
    setState(() {
      _image = frame.image;
      _pixels = pixels;
    });
    AppLog.write(
      'overlay-image-ready',
      '${frame.image.width}x${frame.image.height}',
    );
  }

  @override
  void dispose() {
    _stickerFlyout?.remove();
    _stickerFlyout?.dispose();
    _textController?.dispose();
    _textFocus?.dispose();
    _image?.dispose();
    for (final picture in _stickerPictures) {
      picture.dispose();
    }
    super.dispose();
  }

  Future<void> _loadStickerIcons() async {
    if (_stickerIcons != null) return;
    final resources = await Future.wait([
      rootBundle.loadString('assets/stickers.json'),
      rootBundle.loadString('assets/sticker_zh.json'),
    ]);
    final decoded = Map<String, dynamic>.from(jsonDecode(resources[0]) as Map);
    final translations = Map<String, dynamic>.from(
      jsonDecode(resources[1]) as Map,
    );
    if (mounted) {
      setState(() {
        _stickerIcons = decoded.map(
          (key, value) => MapEntry(key, value as String),
        );
        _stickerZh = Map<String, String>.from(translations['ZH'] as Map);
        _stickerSynonyms = (translations['CN_SYNONYMS'] as Map).map(
          (key, value) =>
              MapEntry(key as String, List<String>.from(value as List)),
        );
      });
    }
  }

  String _stickerLabel(String name) => name
      .split('-')
      .map((part) => _stickerZh[part] ?? part)
      .where((part) => part.isNotEmpty)
      .join();

  bool _stickerMatches(String name, String query) {
    final term = query.trim();
    if (term.isEmpty) return true;
    if (name.contains(term.toLowerCase()) ||
        _stickerLabel(name).contains(term)) {
      return true;
    }
    return (_stickerSynonyms[term] ?? const <String>[]).any(name.contains);
  }

  Future<void> _selectStickerIcon(String name) async {
    final svg = _stickerIcons?[name];
    if (svg == null) return;
    final picture = await vg.loadPicture(
      SvgStringLoader(svg, theme: SvgTheme(currentColor: _stickerColor)),
      context,
    );
    if (!mounted) {
      picture.picture.dispose();
      return;
    }
    _stickerPictures.add(picture.picture);
    setState(() {
      _stickerIconName = name;
      _stickerPicture = picture.picture;
      _stickerPictureSize = picture.size;
    });
  }

  Rect? get _validSelection {
    final rect = _selection;
    return rect != null && rect.width >= 3 && rect.height >= 3 ? rect : null;
  }

  void _queryWindow(Offset point) {
    final now = DateTime.now();
    if (_lastSnap != null && now.difference(_lastSnap!).inMilliseconds < 65)
      return;
    _lastSnap = now;
    final sequence = ++_snapSequence;
    CaptureBridge.windowSnap(_display, point.dx, point.dy)
        .then((rect) {
          if (!mounted || sequence != _snapSequence || _start != null) return;
          final selected = _validSelection;
          final same =
              rect != null &&
              selected != null &&
              (rect.left - selected.left).abs() < 1.5 &&
              (rect.top - selected.top).abs() < 1.5 &&
              (rect.width - selected.width).abs() < 1.5 &&
              (rect.height - selected.height).abs() < 1.5;
          setState(() => _hoverWindow = same ? null : rect);
          if (rect != null) unawaited(_clearOtherOverlays('clearHover'));
        })
        .catchError((Object _) {
          if (mounted && sequence == _snapSequence) {
            setState(() => _hoverWindow = null);
          }
        });
  }

  void _updateHover(Offset point) {
    if (!_hoverLogged) {
      _hoverLogged = true;
      AppLog.write(
        'overlay-first-hover',
        '${point.dx.round()},${point.dy.round()}',
      );
    }
    setState(() => _cursor = point);
    if (_tool == null && !_qrMode) _queryWindow(point);
  }

  Future<void> _clearOtherOverlays(String method) async {
    for (final window in await WindowController.getAll()) {
      if (window.windowId == widget.controller.windowId ||
          windowRole(window) != 'overlay')
        continue;
      try {
        await window.invokeMethod<void>(method);
      } catch (_) {}
    }
  }

  void _resizeSelection(Offset pointer) {
    final start = _resizeOrigin;
    final anchor = _resizePointer;
    final edge = _resizeEdge;
    if (start == null || anchor == null || edge == null) return;
    final dx = pointer.dx - anchor.dx;
    final dy = pointer.dy - anchor.dy;
    final left = start.left + (edge.contains('l') ? dx : 0);
    final right = start.right + (edge.contains('r') ? dx : 0);
    final top = start.top + (edge.contains('t') ? dy : 0);
    final bottom = start.bottom + (edge.contains('b') ? dy : 0);
    final rect = Rect.fromLTRB(
      math.min(left, right),
      math.min(top, bottom),
      math.max(left, right),
      math.max(top, bottom),
    ).intersect(Rect.fromLTWH(0, 0, _display.width, _display.height));
    if (rect.width >= 3 && rect.height >= 3) {
      setState(() => _selection = rect);
    }
  }

  String? get _cursorColor {
    final image = _image;
    final pixels = _pixels;
    final cursor = _cursor;
    if (image == null || pixels == null || cursor == null) return null;
    final x = (cursor.dx * image.width / _display.width).floor().clamp(
      0,
      image.width - 1,
    );
    final y = (cursor.dy * image.height / _display.height).floor().clamp(
      0,
      image.height - 1,
    );
    final index = (y * image.width + x) * 4;
    final r = pixels.getUint8(index).toRadixString(16).padLeft(2, '0');
    final g = pixels.getUint8(index + 1).toRadixString(16).padLeft(2, '0');
    final b = pixels.getUint8(index + 2).toRadixString(16).padLeft(2, '0');
    return '#${r.toUpperCase()}${g.toUpperCase()}${b.toUpperCase()}';
  }

  Future<void> _copyColor() async {
    final color = _cursorColor;
    if (color != null && _tool == null && !_qrMode) {
      await Clipboard.setData(ClipboardData(text: color));
    }
  }

  Future<void> _scanQrRect(Rect rect) async {
    final source = _image;
    if (source == null || rect.width < 3 || rect.height < 3) return;
    setState(() => _qrBusy = true);
    try {
      final bytes = await cropSelection(
        source,
        rect,
        Size(_display.width, _display.height),
      );
      final result = await scanCodes(bytes);
      if (mounted) setState(() => _qrCodes = result);
    } catch (error) {
      if (mounted)
        setState(() => _qrCodes = [DecodedCode('错误', '识别失败：$error', const [])]);
    } finally {
      if (mounted) setState(() => _qrBusy = false);
    }
  }

  Future<void> _recognizeText() async {
    final selection = _validSelection;
    final source = _image;
    if (selection == null || source == null || _ocrBusy) return;
    setState(() => _ocrBusy = true);
    try {
      final bytes = await cropSelection(
        source,
        selection,
        Size(_display.width, _display.height),
      );
      final text = await _recognizeLocally(bytes);
      await showOcrResult(bytes, text);
      await _close();
    } catch (error) {
      if (mounted) setState(() => _ocrText = '识别失败：$error');
    } finally {
      if (mounted) setState(() => _ocrBusy = false);
    }
  }

  Future<String> _recognizeLocally(Uint8List image) async {
    final ball = await findBallWindow();
    if (ball == null) throw StateError('主窗口不可用，无法进行本地文字识别');
    return await ball.invokeMethod<String>('recognizeText', image) ?? '';
  }

  Future<void> _translate() async {
    final selection = _validSelection;
    final source = _image;
    if (selection == null || source == null || _transBusy) return;
    setState(() => _transBusy = true);
    try {
      final bytes = await cropSelection(
        source,
        selection,
        Size(_display.width, _display.height),
      );
      final text = await _recognizeLocally(bytes);
      if (text.trim().isEmpty) {
        if (mounted) setState(() => _ocrText = '这块区域没识别到文字');
        return;
      }
      WindowController? existing;
      for (final window in await WindowController.getAll()) {
        if (windowRole(window) == 'translation') {
          existing = window;
          break;
        }
      }
      if (existing != null) {
        await existing.invokeMethod<void>('openSource', text);
      } else {
        await WindowController.create(
          WindowConfiguration(
            arguments: jsonEncode({'role': 'translation', 'source': text}),
            hiddenAtLaunch: true,
          ),
        );
      }
      await _close();
    } catch (error) {
      if (mounted) setState(() => _ocrText = '翻译失败：$error');
    } finally {
      if (mounted) setState(() => _transBusy = false);
    }
  }

  Future<Uint8List> _crop() async {
    final selection = _validSelection;
    final source = _image;
    if (selection == null || source == null) throw StateError('No selection');
    return cropSelection(
      source,
      selection,
      Size(_display.width, _display.height),
      _annotations,
    );
  }

  void _undo() {
    if (_annotations.isEmpty) return;
    setState(() => _redo.add(_annotations.removeLast()));
  }

  void _redoLast() {
    if (_redo.isEmpty) return;
    setState(() => _annotations.add(_redo.removeLast()));
  }

  Future<void> _requestExit() async {
    if (_closing || _confirmingExit) return;
    _confirmingExit = true;
    final preference = CaptureExitPreference();
    try {
      if (!await preference.shouldAsk()) {
        AppLog.write('capture-exit', 'confirmed-by-preference');
        if (mounted && !_closing) await _close();
        return;
      }
      if (!mounted || _closing) return;
      AppLog.write('capture-exit-confirm-open');
      final route = DialogRoute<CaptureExitDecision>(
        context: context,
        barrierDismissible: false,
        barrierColor: Colors.transparent,
        builder: (_) => const CaptureExitDialog(),
      );
      _exitRoute = route;
      final decision = await Navigator.of(context).push(route);
      _exitRoute = null;
      AppLog.write(
        'capture-exit-confirm-result',
        'exit=${decision?.exit ?? false} dontAsk=${decision?.dontAskAgain ?? false}',
      );
      if (decision?.exit != true || !mounted || _closing) return;
      if (decision!.dontAskAgain) {
        try {
          await preference.stopAsking();
        } catch (error) {
          AppLog.write('capture-exit-preference-failed', error);
        }
      }
      if (mounted && !_closing) await _close();
    } finally {
      _confirmingExit = false;
    }
  }

  Future<void> _close() async {
    if (_closing) return;
    _closing = true;
    final all = await WindowController.getAll();
    for (final window in all) {
      if (window.windowId == widget.controller.windowId) continue;
      if (windowRole(window) == 'ball') {
        debugPrint('SnapPin restore ball: ${window.windowId}');
        await window.invokeMethod<void>('captureFinished');
      } else {
        final role = (jsonDecode(window.arguments) as Map)['role'];
        debugPrint('SnapPin close other window: $role ${window.windowId}');
        if (role == 'overlay') await window.invokeMethod<void>('closeSelf');
      }
    }
    await _closeSelf();
  }

  Future<void> _closeSelf() async {
    _closing = true;
    final exitRoute = _exitRoute;
    if (exitRoute != null && exitRoute.isActive)
      Navigator.of(context).removeRoute(exitRoute);
    _exitRoute = null;
    _finishTextEditor(commit: false);
    _hideStickerFlyout();
    if (_longMode) await windowManager.setIgnoreMouseEvents(false);
    final display = _currentDisplay;
    if (mounted)
      setState(() {
        _currentDisplay = null;
        _image?.dispose();
        _image = null;
        _pixels = null;
      });
    try {
      if (display != null) await File(display.path).delete();
    } catch (_) {}
    await windowManager.hide();
  }

  Future<void> _copy() async {
    await Pasteboard.writeImage(await _crop());
    await _close();
    try {
      await WindowController.create(
        WindowConfiguration(
          arguments: jsonEncode({'role': 'capture-toast'}),
          hiddenAtLaunch: true,
        ),
      );
      AppLog.write('capture-copy-toast', 'shown');
    } catch (error) {
      AppLog.write('capture-copy-toast-failed', error);
    }
  }

  Future<void> _save() async {
    if (!await saveImage(await _crop())) return;
    await _close();
  }

  Future<void> _pin() async {
    final bytes = await _crop();
    final directory = await Directory.systemTemp.createTemp('snappin-pin-');
    final path = '${directory.path}${Platform.pathSeparator}pin.png';
    await File(path).writeAsBytes(bytes);
    final rect = _validSelection!;
    await WindowController.create(
      WindowConfiguration(
        arguments: jsonEncode({
          'role': 'pin',
          'path': path,
          'width': rect.width + 20,
          'height': rect.height + 20,
          'pixelWidth': (rect.width * _display.scale).round(),
          'pixelHeight': (rect.height * _display.scale).round(),
          'x': _display.x + rect.left - 10,
          'y': _display.y + rect.top - 10,
        }),
        hiddenAtLaunch: true,
      ),
    );
    await _close();
  }

  Future<void> _startLongCapture() async {
    final selection = _validSelection;
    final display = _currentDisplay;
    if (selection == null || display == null || _longMode) return;
    final scale = display.scale;
    if (selection.height * scale < 80) {
      setState(() => _ocrText = '选区太矮，长截图至少需要 80px 高');
      return;
    }
    setState(() => _longMode = true);
    await windowManager.setIgnoreMouseEvents(true);
    await Future<void>.delayed(const Duration(milliseconds: 80));
    try {
      final ball = await findBallWindow();
      if (ball == null) throw StateError('主窗口不可用');
      final region = CaptureBridge.longCaptureRegion(display, selection);
      AppLog.write(
        'long-region-coordinates',
        'platform=${Platform.operatingSystem} region=$region scale=$scale',
      );
      await ball.invokeMethod<void>(
        'longCaptureStart',
        jsonEncode({...region, 'overlay': widget.controller.windowId}),
      );
    } catch (error) {
      await windowManager.setIgnoreMouseEvents(false);
      if (mounted)
        setState(() {
          _longMode = false;
          _ocrText = '长截图启动失败：$error';
        });
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_currentDisplay == null) return const SizedBox.shrink();
    if (_longMode) {
      return IgnorePointer(
        child: SizedBox.expand(
          child: CustomPaint(painter: _LongBorderPainter(_validSelection)),
        ),
      );
    }
    final selection = _validSelection;
    final width = _display.width;
    final height = _display.height;
    final barWidth = math.min(916.0, width - 16);
    final barLeft = (width - barWidth) / 2;
    const barTop = 12.0;
    return TooltipTheme(
      data: TooltipThemeData(
        textStyle: const TextStyle(
          fontSize: 12,
          fontWeight: FontWeight.w500,
          color: Colors.white,
        ),
        padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 6),
        decoration: BoxDecoration(
          color: const Color(0xff26313d),
          borderRadius: BorderRadius.circular(5),
        ),
        waitDuration: const Duration(milliseconds: 400),
      ),
      child: Focus(
        autofocus: true,
        onKeyEvent: (_, event) {
          if (event is! KeyDownEvent) return KeyEventResult.ignored;
          if (_editingText != null) return KeyEventResult.ignored;
          if (event.logicalKey == LogicalKeyboardKey.escape) {
            _requestExit();
            return KeyEventResult.handled;
          }
          if (event.logicalKey == LogicalKeyboardKey.enter &&
              selection != null) {
            _copy();
            return KeyEventResult.handled;
          }
          if (event.logicalKey == LogicalKeyboardKey.keyC &&
              _tool == null &&
              !_qrMode) {
            _copyColor();
            return KeyEventResult.handled;
          }
          final modifier =
              HardwareKeyboard.instance.isControlPressed ||
              HardwareKeyboard.instance.isMetaPressed;
          if (modifier && event.logicalKey == LogicalKeyboardKey.keyZ) {
            HardwareKeyboard.instance.isShiftPressed ? _redoLast() : _undo();
            return KeyEventResult.handled;
          }
          if (modifier && event.logicalKey == LogicalKeyboardKey.keyY) {
            _redoLast();
            return KeyEventResult.handled;
          }
          return KeyEventResult.ignored;
        },
        child: MouseRegion(
          cursor: SystemMouseCursors.precise,
          onExit: (event) {
            final outside = !(Offset.zero & Size(width, height)).contains(
              event.localPosition,
            );
            AppLog.write(
              'overlay-pointer-exit',
              'outside=$outside selection=${_validSelection != null}',
            );
            if (!outside) return;
            _snapSequence++;
            if (mounted) {
              setState(() {
                _cursor = null;
                _hoverWindow = null;
              });
            }
          },
          child: Stack(
            children: [
              Positioned.fill(
                child: Image.file(File(_display.path), fit: BoxFit.fill),
              ),
              Positioned.fill(
                child: Listener(
                  behavior: HitTestBehavior.opaque,
                  onPointerHover: (event) {
                    if (_closing || _currentDisplay == null) return;
                    _updateHover(_withinDisplay(event.localPosition));
                  },
                  onPointerDown: (event) {
                    if (_closing || _currentDisplay == null) return;
                    AppLog.write(
                      'overlay-pointer-down',
                      'buttons=${event.buttons} selection=${_validSelection != null} '
                          'snap=${_hoverWindow != null}',
                    );
                    if (event.buttons == 2) {
                      if (_validSelection == null) {
                        _close();
                      } else {
                        setState(() {
                          _selection = null;
                          _annotations.clear();
                          _redo.clear();
                          _draft = null;
                          _tool = null;
                          _qrMode = false;
                          _qrRect = null;
                          _qrCodes = null;
                          _ocrText = null;
                        });
                      }
                      _start = null;
                      return;
                    }
                    if (event.buttons != 1) return;
                    final point = _withinDisplay(event.localPosition);
                    if (_validSelection?.contains(point) == true &&
                        _tool == null &&
                        !_qrMode) {
                      final now = DateTime.now();
                      final previous = _lastSelectionClick;
                      final previousPoint = _lastSelectionClickPoint;
                      _lastSelectionClick = now;
                      _lastSelectionClickPoint = point;
                      if (previous != null &&
                          previousPoint != null &&
                          now.difference(previous).inMilliseconds <= 350 &&
                          (point - previousPoint).distance <= 6) {
                        AppLog.write('overlay-selection-double-click');
                        unawaited(_copy());
                      }
                      return;
                    }
                    _lastSelectionClick = null;
                    if (_validSelection?.contains(point) == true && _qrMode) {
                      _qrStart = point;
                      setState(() {
                        _qrRect = Rect.fromPoints(point, point);
                        _qrCodes = null;
                      });
                      return;
                    }
                    if (_validSelection?.contains(point) == true &&
                        _tool != null) {
                      final activeTool = _tool!;
                      setState(
                        () => _draft = Annotation(
                          tool: activeTool,
                          points: [point],
                          color: _color,
                          strokeWidth: activeTool == AnnotationTool.sticker
                              ? _stickerSize
                              : activeTool == AnnotationTool.text ||
                                    activeTool == AnnotationTool.watermark ||
                                    activeTool == AnnotationTool.badge
                              ? _fontSize
                              : _strokeWidth,
                          text: activeTool == AnnotationTool.sticker
                              ? _sticker
                              : _annotationText,
                          stickerPicture: activeTool == AnnotationTool.sticker
                              ? _stickerPicture
                              : null,
                          stickerPictureSize:
                              activeTool == AnnotationTool.sticker
                              ? _stickerPictureSize
                              : null,
                          number: _badgeNumber,
                          parameter: activeTool == AnnotationTool.mosaic
                              ? _mosaicBlock
                              : activeTool == AnnotationTool.spotlight
                              ? _spotDim
                              : activeTool == AnnotationTool.eraser
                              ? _eraserRadius
                              : 0,
                        ),
                      );
                      return;
                    }
                    _start = point;
                    unawaited(_clearOtherOverlays('clearSelection'));
                    _pressedWindow = _hoverWindow;
                    setState(() {
                      _selection = null;
                      _hoverWindow = null;
                      _cursor = _start;
                      _annotations.clear();
                      _redo.clear();
                      _draft = null;
                      _tool = null;
                      _qrMode = false;
                      _qrRect = null;
                      _qrCodes = null;
                      _ocrText = null;
                    });
                  },
                  onPointerMove: (event) {
                    if (_closing || _currentDisplay == null) return;
                    final point = _withinDisplay(event.localPosition);
                    if (_qrStart != null && _validSelection != null) {
                      final bounds = _validSelection!;
                      final bounded = Offset(
                        point.dx.clamp(bounds.left, bounds.right),
                        point.dy.clamp(bounds.top, bounds.bottom),
                      );
                      setState(
                        () => _qrRect = Rect.fromPoints(_qrStart!, bounded),
                      );
                      return;
                    }
                    if (_draft != null) {
                      final prior = _draft!;
                      setState(
                        () => _draft = Annotation(
                          tool: prior.tool,
                          points:
                              prior.tool == AnnotationTool.pen ||
                                  prior.tool == AnnotationTool.highlighter ||
                                  prior.tool == AnnotationTool.eraser
                              ? [...prior.points, point]
                              : [prior.points.first, point],
                          color: prior.color,
                          strokeWidth: prior.strokeWidth,
                          text: prior.text,
                          number: prior.number,
                          parameter: prior.parameter,
                          stickerPicture: prior.stickerPicture,
                          stickerPictureSize: prior.stickerPictureSize,
                        ),
                      );
                      return;
                    }
                    if (_start == null) {
                      _updateHover(point);
                    } else {
                      if ((point - _start!).distance > 4) _pressedWindow = null;
                      setState(() {
                        _cursor = point;
                        _selection = Rect.fromPoints(_start!, point);
                      });
                    }
                  },
                  onPointerUp: (event) {
                    if (_closing || _currentDisplay == null) return;
                    if (_qrStart != null) {
                      _qrStart = null;
                      final rect = _qrRect;
                      if (rect != null) _scanQrRect(rect);
                    }
                    if (_draft != null) {
                      unawaited(_commitDraft());
                    }
                    if (_start != null && _pressedWindow != null) {
                      setState(() => _selection = _pressedWindow);
                    }
                    if (_validSelection == null) {
                      setState(
                        () => _cursor = _withinDisplay(event.localPosition),
                      );
                    }
                    AppLog.write(
                      'overlay-pointer-up',
                      'selection=${_validSelection != null} '
                          'cursor=${_cursor != null}',
                    );
                    _start = null;
                    _pressedWindow = null;
                  },
                  child: CustomPaint(
                    painter: _SelectionPainter(
                      selection: _selection,
                      annotations: List.of(_annotations),
                      draft: _draft,
                      qrRect: _qrRect,
                    ),
                  ),
                ),
              ),
              if (_hoverWindow != null &&
                  _start == null &&
                  _tool == null &&
                  !_qrMode)
                Positioned.fromRect(
                  rect: _hoverWindow!,
                  child: IgnorePointer(
                    child: Container(
                      decoration: BoxDecoration(
                        border: Border.all(
                          color: const Color(0xff1688f0),
                          width: 3,
                        ),
                        borderRadius: BorderRadius.circular(2),
                      ),
                    ),
                  ),
                ),
              if (_hoverWindow != null &&
                  _start == null &&
                  _tool == null &&
                  !_qrMode)
                Positioned(
                  left: _hoverWindow!.left.clamp(
                    0.0,
                    math.max(0.0, width - 90),
                  ),
                  top: _hoverWindow!.top >= 25
                      ? _hoverWindow!.top - 25
                      : _hoverWindow!.bottom + 3,
                  child: IgnorePointer(
                    child: Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 7,
                        vertical: 3,
                      ),
                      color: const Color(0xff1688f0),
                      child: Text(
                        '${(_hoverWindow!.width * _display.scale).round()} × '
                        '${(_hoverWindow!.height * _display.scale).round()}',
                        style: const TextStyle(
                          color: Colors.white,
                          fontSize: 11,
                        ),
                      ),
                    ),
                  ),
                ),
              if (_cursor != null &&
                  _image != null &&
                  _tool == null &&
                  !_qrMode)
                Positioned(
                  left:
                      (_cursor!.dx + 22 + 156 > width
                              ? _cursor!.dx - 178
                              : _cursor!.dx + 22)
                          .clamp(0.0, math.max(0.0, width - 156)),
                  top:
                      (_cursor!.dy + 22 + 180 > height
                              ? _cursor!.dy - 202
                              : _cursor!.dy + 22)
                          .clamp(0.0, math.max(0.0, height - 180)),
                  child: IgnorePointer(
                    child: CustomPaint(
                      size: const Size(156, 180),
                      painter: _MagnifierPainter(
                        image: _image!,
                        position: _cursor!,
                        displaySize: Size(width, height),
                        color: _cursorColor ?? '#000000',
                      ),
                    ),
                  ),
                ),
              if (selection != null)
                Positioned(
                  left: selection.left.clamp(8.0, math.max(8.0, width - 120)),
                  top: (selection.top >= 32
                      ? selection.top - 27
                      : selection.bottom + 4),
                  child: IgnorePointer(
                    child: Container(
                      padding: const EdgeInsets.symmetric(
                        horizontal: 7,
                        vertical: 3,
                      ),
                      decoration: BoxDecoration(
                        color: const Color(0xcc1d232b),
                        borderRadius: BorderRadius.circular(3),
                      ),
                      child: Text(
                        '${(selection.width * _display.scale).round()} × '
                        '${(selection.height * _display.scale).round()}',
                        style: const TextStyle(
                          color: Colors.white,
                          fontSize: 11,
                        ),
                      ),
                    ),
                  ),
                ),
              if (selection != null && _tool == null && !_qrMode) ...[
                for (final edge in const [
                  't',
                  'r',
                  'b',
                  'l',
                  'tl',
                  'tr',
                  'br',
                  'bl',
                ])
                  _selectionHandle(selection, edge),
              ],
              if (selection != null)
                Positioned(
                  left: barLeft,
                  top: barTop,
                  child: Container(
                    width: barWidth,
                    height: 48,
                    padding: const EdgeInsets.symmetric(horizontal: 4),
                    decoration: BoxDecoration(
                      color: const Color(0xfffafafa),
                      border: Border.all(color: const Color(0x120f172a)),
                      borderRadius: BorderRadius.circular(12),
                      boxShadow: const [
                        BoxShadow(
                          color: Color(0x380f172a),
                          blurRadius: 24,
                          offset: Offset(0, 6),
                        ),
                        BoxShadow(
                          color: Color(0x190f172a),
                          blurRadius: 3,
                          offset: Offset(0, 1),
                        ),
                      ],
                    ),
                    child: MouseRegion(
                      onEnter: (_) {
                        _snapSequence++;
                        setState(() => _hoverWindow = null);
                      },
                      child: SingleChildScrollView(
                        scrollDirection: Axis.horizontal,
                        child: Row(
                          mainAxisAlignment: MainAxisAlignment.start,
                          children: [
                            _toolButton(
                              'rectangle',
                              '矩形',
                              AnnotationTool.rectangle,
                            ),
                            _toolButton(
                              'ellipse',
                              '椭圆',
                              AnnotationTool.ellipse,
                            ),
                            _toolButton(
                              'sticker',
                              '贴纸',
                              AnnotationTool.sticker,
                            ),
                            _toolButton('arrow', '箭头', AnnotationTool.arrow),
                            _toolButton('pen', '画笔', AnnotationTool.pen),
                            _toolButton('mosaic', '马赛克', AnnotationTool.mosaic),
                            _toolButton(
                              'spotlight',
                              '聚光灯',
                              AnnotationTool.spotlight,
                            ),
                            _toolButton('text', '文字', AnnotationTool.text),
                            _toolButton('badge', '序号', AnnotationTool.badge),
                            _toolButton('eraser', '橡皮擦', AnnotationTool.eraser),
                            _toolButton(
                              'watermark',
                              '水印',
                              AnnotationTool.watermark,
                            ),
                            _toolButton(
                              'smart-erase',
                              '智能擦除',
                              AnnotationTool.smartErase,
                            ),
                            const VerticalDivider(
                              width: 13,
                              thickness: 1,
                              color: Color(0xffe2e2e2),
                              indent: 9,
                              endIndent: 9,
                            ),
                            _actionButton(
                              'translate',
                              '翻译（框选后一键识别并翻译，左原文右译文）',
                              _transBusy ? null : _translate,
                              active: _transBusy,
                            ),
                            _actionButton(
                              'ocr',
                              '文字识别（识别当前选区内的文字，支持中文）',
                              _ocrBusy ? null : _recognizeText,
                              active: _ocrBusy,
                            ),
                            Tooltip(
                              message: '二维码识别（拖选区域，支持一图多码）',
                              child: Material(
                                color: _qrMode
                                    ? const Color(0x160f172a)
                                    : Colors.transparent,
                                borderRadius: BorderRadius.circular(9),
                                child: InkWell(
                                  borderRadius: BorderRadius.circular(9),
                                  onTap: () => setState(() {
                                    _qrMode = !_qrMode;
                                    _tool = null;
                                  }),
                                  child: SizedBox(
                                    width: 40,
                                    height: 46,
                                    child: Center(
                                      child: ShotIcon(
                                        'qr',
                                        size: 22,
                                        color: _qrMode
                                            ? const Color(0xff1f2937)
                                            : const Color(0xff4c4c4c),
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                            _actionButton(
                              'long',
                              '长截图（滚动页面，实时预览；Esc 完成）',
                              _startLongCapture,
                            ),
                            const VerticalDivider(
                              width: 13,
                              thickness: 1,
                              color: Color(0xffe2e2e2),
                              indent: 9,
                              endIndent: 9,
                            ),
                            _actionButton(
                              'undo',
                              '撤销 (Ctrl+Z)',
                              _annotations.isEmpty ? null : _undo,
                            ),
                            _actionButton(
                              'redo',
                              '重做 (Ctrl+Shift+Z / Ctrl+Y)',
                              _redo.isEmpty ? null : _redoLast,
                            ),
                            _actionButton('save', '保存到文件', _save),
                            _actionButton('pin', '贴图到屏幕', _pin),
                            _actionButton('close', '取消截图 (Esc)', _close),
                            _actionButton('check', '完成并复制 (Enter)', _copy),
                          ],
                        ),
                      ),
                    ),
                  ),
                ),
              if (selection != null && (_tool != null || _qrMode))
                Positioned(
                  left: barLeft,
                  top: barTop + 54,
                  child: Container(
                    height: 40,
                    constraints: BoxConstraints(maxWidth: width - 16),
                    padding: const EdgeInsets.symmetric(horizontal: 12),
                    decoration: BoxDecoration(
                      color: const Color(0xfffafafa),
                      border: Border.all(color: const Color(0x120f172a)),
                      borderRadius: BorderRadius.circular(10),
                      boxShadow: const [
                        BoxShadow(
                          color: Color(0x2e0f172a),
                          blurRadius: 20,
                          offset: Offset(0, 6),
                        ),
                      ],
                    ),
                    child: SingleChildScrollView(
                      scrollDirection: Axis.horizontal,
                      child: _toolProperties(),
                    ),
                  ),
                ),
              if (selection != null && (_qrBusy || _qrCodes != null))
                Positioned(
                  left: barLeft,
                  top: (barTop + (_qrMode ? 100 : 54))
                      .clamp(8.0, math.max(8.0, height - 270))
                      .toDouble(),
                  child: Container(
                    width: math.min(420, width - 16),
                    constraints: const BoxConstraints(maxHeight: 260),
                    padding: const EdgeInsets.symmetric(
                      horizontal: 10,
                      vertical: 8,
                    ),
                    color: const Color(0xfffafafa),
                    child: SingleChildScrollView(
                      child: Column(
                        mainAxisSize: MainAxisSize.min,
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          if (_qrBusy)
                            const Text('识别中…', style: TextStyle(fontSize: 12)),
                          if (!_qrBusy && _qrCodes?.isEmpty == true)
                            const Text(
                              '未识别到二维码/条码',
                              style: TextStyle(fontSize: 12),
                            ),
                          for (final code in _qrCodes ?? const <DecodedCode>[])
                            Row(
                              children: [
                                Text(
                                  code.format,
                                  style: const TextStyle(
                                    fontSize: 10,
                                    color: Color(0xff6b7280),
                                  ),
                                ),
                                const SizedBox(width: 8),
                                Expanded(
                                  child: SelectableText(
                                    code.text,
                                    style: const TextStyle(fontSize: 12),
                                  ),
                                ),
                                IconButton(
                                  tooltip: '复制',
                                  onPressed: () => Clipboard.setData(
                                    ClipboardData(text: code.text),
                                  ),
                                  icon: const Icon(Icons.copy, size: 16),
                                ),
                                if (_codeUrl(code.text) != null)
                                  IconButton(
                                    tooltip: '打开链接',
                                    onPressed: () => Process.start(
                                      Platform.isMacOS
                                          ? 'open'
                                          : 'explorer.exe',
                                      [code.text],
                                    ),
                                    icon: const Icon(
                                      Icons.open_in_new,
                                      size: 16,
                                    ),
                                  ),
                              ],
                            ),
                        ],
                      ),
                    ),
                  ),
                ),
              if (selection != null && (_ocrBusy || _ocrText != null))
                Positioned(
                  left: barLeft,
                  top: (barTop + ((_tool != null || _qrMode) ? 100 : 54))
                      .clamp(8.0, math.max(8.0, height - 166))
                      .toDouble(),
                  child: Container(
                    width: math.min(420, width - 16),
                    constraints: const BoxConstraints(maxHeight: 150),
                    padding: const EdgeInsets.all(10),
                    color: const Color(0xfffafafa),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Flexible(
                          child: SizedBox(
                            height: 96,
                            child: SingleChildScrollView(
                              child: Text(
                                _ocrBusy ? '识别中…' : _ocrText!,
                                style: const TextStyle(fontSize: 12),
                              ),
                            ),
                          ),
                        ),
                        if (!_ocrBusy && _ocrText != '未识别到文字')
                          Align(
                            alignment: Alignment.centerRight,
                            child: TextButton.icon(
                              onPressed: () => Clipboard.setData(
                                ClipboardData(text: _ocrText!),
                              ),
                              icon: const Icon(Icons.copy, size: 15),
                              label: const Text('复制'),
                            ),
                          ),
                      ],
                    ),
                  ),
                ),
              if (_editingText != null) _textEditor(Size(width, height)),
            ],
          ),
        ),
      ),
    );
  }

  Uri? _codeUrl(String text) {
    final uri = Uri.tryParse(text);
    return uri != null && (uri.scheme == 'http' || uri.scheme == 'https')
        ? uri
        : null;
  }

  Future<void> _commitDraft() async {
    final draft = _draft;
    if (draft == null) return;
    setState(() => _draft = null);
    if (draft.tool == AnnotationTool.text ||
        draft.tool == AnnotationTool.watermark) {
      _beginTextEditor(draft);
      return;
    }
    setState(() {
      _annotations.add(draft);
      if (draft.tool == AnnotationTool.badge) _badgeNumber++;
      _redo.clear();
    });
  }

  void _beginTextEditor(Annotation draft) {
    _finishTextEditor();
    final controller = TextEditingController(
      text: draft.tool == AnnotationTool.watermark ? 'SnapPin' : '',
    );
    final focus = FocusNode();
    setState(() {
      _editingText = draft;
      _textController = controller;
      _textFocus = focus;
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted && _textFocus == focus) focus.requestFocus();
    });
  }

  void _finishTextEditor({bool commit = true}) {
    final draft = _editingText;
    if (draft == null) return;
    final controller = _textController;
    final focus = _textFocus;
    final value = controller?.text ?? '';
    setState(() {
      _editingText = null;
      _textController = null;
      _textFocus = null;
      if (commit && value.trim().isNotEmpty) {
        _annotationText = value;
        _annotations.add(
          Annotation(
            tool: draft.tool,
            points: draft.points,
            color: draft.color,
            strokeWidth: draft.strokeWidth,
            text: value,
          ),
        );
        _redo.clear();
      }
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      controller?.dispose();
      focus?.dispose();
    });
  }

  Widget _textEditor(Size screen) {
    final draft = _editingText!;
    final controller = _textController!;
    final focus = _textFocus!;
    final point = draft.points.first;
    final style = TextStyle(
      color: draft.color,
      fontSize: draft.strokeWidth,
      fontFamily: 'Segoe UI',
      height: 1.32,
    );
    final longest = controller.text
        .split('\n')
        .fold<String>(
          '',
          (current, line) => line.length > current.length ? line : current,
        );
    final measured = TextPainter(
      text: TextSpan(text: longest.isEmpty ? ' ' : longest, style: style),
      textDirection: TextDirection.ltr,
    )..layout();
    final left = point.dx.clamp(4.0, math.max(4.0, screen.width - 148));
    final top = point.dy.clamp(4.0, math.max(4.0, screen.height - 48));
    final available = math.max(140.0, screen.width - left - 8);
    final editorWidth = (measured.width + 8).clamp(140.0, available);
    return Positioned(
      left: left - 3,
      top: top - 3,
      child: CustomPaint(
        painter: _TextEditorBorderPainter(),
        child: Padding(
          padding: const EdgeInsets.all(3),
          child: SizedBox(
            width: editorWidth,
            child: ConstrainedBox(
              constraints: BoxConstraints(maxHeight: screen.height - top - 8),
              child: Focus(
                onFocusChange: (focused) {
                  if (!focused && _textFocus == focus) _finishTextEditor();
                },
                onKeyEvent: (_, event) {
                  if (event is! KeyDownEvent) return KeyEventResult.ignored;
                  final composing = controller.value.composing;
                  if (composing.isValid && !composing.isCollapsed) {
                    return KeyEventResult.ignored;
                  }
                  if (event.logicalKey == LogicalKeyboardKey.escape) {
                    _finishTextEditor(commit: false);
                    return KeyEventResult.handled;
                  }
                  if (event.logicalKey == LogicalKeyboardKey.enter &&
                      (HardwareKeyboard.instance.isControlPressed ||
                          HardwareKeyboard.instance.isMetaPressed)) {
                    _finishTextEditor();
                    return KeyEventResult.handled;
                  }
                  return KeyEventResult.ignored;
                },
                child: TextField(
                  controller: controller,
                  focusNode: focus,
                  autofocus: true,
                  maxLines: null,
                  minLines: 1,
                  keyboardType: TextInputType.multiline,
                  style: style,
                  cursorColor: draft.color,
                  decoration: const InputDecoration.collapsed(hintText: ''),
                  onChanged: (_) => setState(() {}),
                  onTapOutside: (_) => _finishTextEditor(),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  void _hideStickerFlyout() {
    _stickerFlyout?.remove();
    _stickerFlyout?.dispose();
    _stickerFlyout = null;
    if (mounted) {
      setState(() {
        _stickerColorOpen = false;
        _stickerSizeOpen = false;
      });
    }
  }

  void _showStickerFlyout(BuildContext buttonContext, {required bool color}) {
    if (color ? _stickerColorOpen : _stickerSizeOpen) {
      _hideStickerFlyout();
      return;
    }
    final box = buttonContext.findRenderObject() as RenderBox;
    final origin = box.localToGlobal(Offset.zero);
    final screen = MediaQuery.sizeOf(buttonContext);
    final flyoutWidth = color ? 238.0 : 190.0;
    final flyoutHeight = color ? 32.0 : 40.0;
    final left = (origin.dx + box.size.width + 6)
        .clamp(8.0, math.max(8.0, screen.width - flyoutWidth - 8))
        .toDouble();
    final top = (origin.dy + box.size.height / 2 - flyoutHeight / 2)
        .clamp(8.0, math.max(8.0, screen.height - flyoutHeight - 8))
        .toDouble();
    _hideStickerFlyout();
    setState(() {
      _stickerColorOpen = color;
      _stickerSizeOpen = !color;
    });
    _stickerFlyout = OverlayEntry(
      builder: (overlayContext) => Positioned(
        left: left,
        top: top,
        child: TapRegion(
          groupId: _stickerFlyoutGroup,
          onTapOutside: (_) => _hideStickerFlyout(),
          child: Material(
            color: const Color(0xfffafafa),
            elevation: 8,
            borderRadius: BorderRadius.circular(8),
            child: Container(
              width: flyoutWidth,
              height: flyoutHeight,
              padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 6),
              decoration: BoxDecoration(
                border: Border.all(color: const Color(0x120f172a)),
                borderRadius: BorderRadius.circular(8),
              ),
              child: color
                  ? Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        for (final value in const [
                          Color(0xff111827),
                          Color(0xffef4444),
                          Color(0xfff97316),
                          Color(0xffeab308),
                          Color(0xff22c55e),
                          Color(0xff3b82f6),
                          Color(0xff8b5cf6),
                          Color(0xffec4899),
                          Color(0xffffffff),
                        ])
                          Tooltip(
                            message:
                                '#${value.toARGB32().toRadixString(16).substring(2)}',
                            child: InkWell(
                              customBorder: const CircleBorder(),
                              onTap: () {
                                setState(() => _stickerColor = value);
                                if (_stickerIconName != null) {
                                  unawaited(
                                    _selectStickerIcon(_stickerIconName!),
                                  );
                                }
                                _hideStickerFlyout();
                              },
                              child: Container(
                                width: 18,
                                height: 18,
                                decoration: BoxDecoration(
                                  color: value,
                                  shape: BoxShape.circle,
                                  border: Border.all(
                                    color: const Color(0x2e0f172a),
                                  ),
                                  boxShadow: _stickerColor == value
                                      ? const [
                                          BoxShadow(
                                            color: Color(0xff12a5d8),
                                            spreadRadius: 2,
                                          ),
                                        ]
                                      : null,
                                ),
                              ),
                            ),
                          ),
                      ],
                    )
                  : StatefulBuilder(
                      builder: (context, update) => Row(
                        children: [
                          SizedBox(
                            width: 34,
                            child: Text(
                              '${_stickerSize.round()}px',
                              textAlign: TextAlign.right,
                              style: const TextStyle(fontSize: 12),
                            ),
                          ),
                          const SizedBox(width: 7),
                          SizedBox(
                            width: 128,
                            child: SliderTheme(
                              data: SliderTheme.of(context).copyWith(
                                trackHeight: 3,
                                thumbShape: const RoundSliderThumbShape(
                                  enabledThumbRadius: 6,
                                ),
                              ),
                              child: Slider(
                                min: 16,
                                max: 160,
                                divisions: 144,
                                value: _stickerSize,
                                activeColor: const Color(0xff12a5d8),
                                onChanged: (value) {
                                  setState(() => _stickerSize = value);
                                  update(() {});
                                },
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
    );
    Overlay.of(buttonContext).insert(_stickerFlyout!);
  }

  Widget _toolProperties() {
    if (_qrMode) {
      return const Text(
        '拖选要识别的区域，支持一图多码',
        style: TextStyle(fontSize: 11, color: Color(0xff6b7280)),
      );
    }
    final tool = _tool;
    if (tool == AnnotationTool.mosaic) {
      return _segments([
        for (final item in const [(6.0, '小'), (12.0, '中'), (20.0, '大')])
          (
            item.$2,
            _mosaicBlock == item.$1,
            () => setState(() => _mosaicBlock = item.$1),
          ),
      ]);
    }
    if (tool == AnnotationTool.spotlight) {
      return _propertySlider(
        '暗度',
        _spotDim,
        40,
        230,
        '${(_spotDim / 255 * 100).round()}%',
        (value) => setState(() => _spotDim = value),
      );
    }
    if (tool == AnnotationTool.eraser) {
      return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          _propertySlider(
            '笔尖',
            _eraserRadius,
            4,
            48,
            '${_eraserRadius.round()}px',
            (value) => setState(() => _eraserRadius = value),
          ),
          const SizedBox(width: 10),
          const Text(
            '按住拖过已画的标注即可擦除（不动原图）',
            style: TextStyle(fontSize: 11, color: Color(0xff6b7280)),
          ),
        ],
      );
    }
    if (tool == AnnotationTool.smartErase) {
      return const Text(
        '拖选要抹掉的区域，自动用周围画面补全',
        style: TextStyle(fontSize: 11, color: Color(0xff6b7280)),
      );
    }
    if (tool == AnnotationTool.sticker) {
      return Row(
        mainAxisSize: MainAxisSize.min,
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
            Padding(
              padding: const EdgeInsets.only(right: 2),
              child: InkWell(
                onTap: () => setState(() {
                  _sticker = glyph;
                  _stickerIconName = null;
                  _stickerPicture = null;
                  _stickerPictureSize = null;
                }),
                child: Container(
                  width: 26,
                  height: 26,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: _stickerIconName == null && _sticker == glyph
                        ? const Color(0x1f12a5d8)
                        : Colors.transparent,
                    border: Border.all(
                      color: _stickerIconName == null && _sticker == glyph
                          ? const Color(0xff12a5d8)
                          : Colors.transparent,
                    ),
                    borderRadius: BorderRadius.circular(6),
                  ),
                  child: Text(glyph, style: const TextStyle(fontSize: 16)),
                ),
              ),
            ),
          const SizedBox(width: 10),
          Builder(
            builder: (buttonContext) => TapRegion(
              groupId: _stickerFlyoutGroup,
              child: Tooltip(
                message: '贴纸颜色',
                child: InkWell(
                  onTap: () => _showStickerFlyout(buttonContext, color: true),
                  child: Container(
                    width: 30,
                    height: 28,
                    decoration: BoxDecoration(
                      color: _stickerColorOpen
                          ? const Color(0x1f12a5d8)
                          : Colors.transparent,
                      border: Border.all(
                        color: _stickerColorOpen
                            ? const Color(0xff12a5d8)
                            : Colors.transparent,
                      ),
                      borderRadius: BorderRadius.circular(6),
                    ),
                    child: Center(
                      child: SvgPicture.asset(
                        'assets/icons/shot/color-picker.svg',
                        width: 22,
                        height: 22,
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ),
          const SizedBox(width: 10),
          Builder(
            builder: (buttonContext) => TapRegion(
              groupId: _stickerFlyoutGroup,
              child: Tooltip(
                message: '贴纸尺寸 ${_stickerSize.round()}px',
                child: InkWell(
                  onTap: () => _showStickerFlyout(buttonContext, color: false),
                  child: Container(
                    constraints: const BoxConstraints(minWidth: 30),
                    height: 28,
                    padding: const EdgeInsets.symmetric(horizontal: 5),
                    alignment: Alignment.center,
                    decoration: BoxDecoration(
                      color: _stickerSizeOpen
                          ? const Color(0x1f12a5d8)
                          : Colors.transparent,
                      border: Border.all(
                        color: _stickerSizeOpen
                            ? const Color(0xff12a5d8)
                            : Colors.transparent,
                      ),
                      borderRadius: BorderRadius.circular(6),
                    ),
                    child: Text(
                      '${_stickerSize.round()}px',
                      style: const TextStyle(fontSize: 12),
                    ),
                  ),
                ),
              ),
            ),
          ),
          const SizedBox(width: 10),
          SizedBox(
            width: 190,
            height: 27,
            child: RawAutocomplete<String>(
              optionsBuilder: (query) {
                final names = _stickerIcons?.keys ?? const <String>[];
                final term = query.text.trim().toLowerCase();
                return names
                    .where((name) => _stickerMatches(name, term))
                    .take(60);
              },
              onSelected: (name) => unawaited(_selectStickerIcon(name)),
              fieldViewBuilder: (context, controller, focus, submit) =>
                  TextField(
                    controller: controller,
                    focusNode: focus,
                    style: const TextStyle(fontSize: 12),
                    decoration: InputDecoration(
                      hintText: _stickerIcons == null
                          ? '加载图标…'
                          : '搜索 ${_stickerIcons!.length} 个图标',
                      isDense: true,
                      filled: true,
                      fillColor: Colors.white,
                      hintStyle: const TextStyle(
                        fontSize: 12,
                        color: Color(0xff9299a2),
                      ),
                      contentPadding: const EdgeInsets.symmetric(
                        horizontal: 8,
                        vertical: 5,
                      ),
                      enabledBorder: OutlineInputBorder(
                        borderRadius: BorderRadius.circular(6),
                        borderSide: const BorderSide(
                          color: Color(0xffd4d9de),
                          width: 1,
                        ),
                      ),
                      focusedBorder: OutlineInputBorder(
                        borderRadius: BorderRadius.circular(6),
                        borderSide: const BorderSide(
                          color: Color(0xff12a5d8),
                          width: 1,
                        ),
                      ),
                    ),
                  ),
              optionsViewBuilder: (context, select, options) =>
                  Transform.translate(
                    offset: const Offset(-74, 6),
                    child: Align(
                      alignment: Alignment.topLeft,
                      child: Material(
                        color: const Color(0xfffafafa),
                        elevation: 8,
                        borderRadius: BorderRadius.circular(8),
                        child: SizedBox(
                          width: 264,
                          height: 204,
                          child: GridView.count(
                            padding: const EdgeInsets.all(6),
                            crossAxisCount: 6,
                            children: [
                              for (final name in options)
                                Tooltip(
                                  message: '${_stickerLabel(name)}  $name',
                                  child: InkWell(
                                    onTap: () => select(name),
                                    child: Container(
                                      decoration: BoxDecoration(
                                        color: _stickerIconName == name
                                            ? const Color(0x1f12a5d8)
                                            : Colors.transparent,
                                        borderRadius: BorderRadius.circular(6),
                                        border: Border.all(
                                          color: _stickerIconName == name
                                              ? const Color(0xff12a5d8)
                                              : Colors.transparent,
                                        ),
                                      ),
                                      child: Center(
                                        child: SvgPicture.string(
                                          _stickerIcons![name]!,
                                          width: 18,
                                          height: 18,
                                          theme: SvgTheme(
                                            currentColor: _stickerColor,
                                          ),
                                        ),
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
          ),
        ],
      );
    }
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (tool == AnnotationTool.pen ||
            tool == AnnotationTool.highlighter) ...[
          _segments([
            (
              '画笔',
              tool == AnnotationTool.pen,
              () => setState(() => _tool = AnnotationTool.pen),
            ),
            (
              '荧光笔',
              tool == AnnotationTool.highlighter,
              () => setState(() => _tool = AnnotationTool.highlighter),
            ),
          ]),
          const SizedBox(width: 10),
        ],
        for (final color in const [
          Color(0xffe62d2d),
          Color(0xffff9628),
          Color(0xfffacc15),
          Color(0xff22c55e),
          Color(0xff3b82f6),
          Color(0xffffffff),
          Color(0xff0f172a),
        ])
          Padding(
            padding: const EdgeInsets.only(right: 6),
            child: Tooltip(
              message: '颜色',
              child: InkWell(
                onTap: () => setState(() => _color = color),
                child: Container(
                  width: 18,
                  height: 18,
                  decoration: BoxDecoration(
                    color: color,
                    shape: BoxShape.circle,
                    border: Border.all(color: const Color(0x291f2937)),
                    boxShadow: color == _color
                        ? const [
                            BoxShadow(
                              color: Color(0xff4c4c4c),
                              spreadRadius: 3.5,
                            ),
                            BoxShadow(
                              color: Color(0xfffafafa),
                              spreadRadius: 2,
                            ),
                          ]
                        : null,
                  ),
                ),
              ),
            ),
          ),
        Container(width: 1, height: 20, color: const Color(0xffe2e2e2)),
        const SizedBox(width: 8),
        _propertySlider(
          '',
          tool == AnnotationTool.text ||
                  tool == AnnotationTool.badge ||
                  tool == AnnotationTool.watermark
              ? _fontSize
              : _strokeWidth,
          tool == AnnotationTool.text ||
                  tool == AnnotationTool.badge ||
                  tool == AnnotationTool.watermark
              ? 10
              : 1,
          tool == AnnotationTool.text ||
                  tool == AnnotationTool.badge ||
                  tool == AnnotationTool.watermark
              ? 72
              : 24,
          '${(tool == AnnotationTool.text || tool == AnnotationTool.badge || tool == AnnotationTool.watermark ? _fontSize : _strokeWidth).round()}px',
          (value) => setState(() {
            if (tool == AnnotationTool.text ||
                tool == AnnotationTool.badge ||
                tool == AnnotationTool.watermark) {
              _fontSize = value;
            } else {
              _strokeWidth = value;
            }
          }),
        ),
        if (tool == AnnotationTool.watermark) ...[
          const SizedBox(width: 10),
          const Text(
            '拖选区域后输入水印文字',
            style: TextStyle(fontSize: 11, color: Color(0xff6b7280)),
          ),
        ],
      ],
    );
  }

  Widget _segments(List<(String, bool, VoidCallback)> options) => Container(
    padding: const EdgeInsets.all(2),
    decoration: BoxDecoration(
      color: const Color(0x100f172a),
      borderRadius: BorderRadius.circular(7),
    ),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final option in options)
          InkWell(
            onTap: option.$3,
            child: Container(
              height: 24,
              alignment: Alignment.center,
              padding: const EdgeInsets.symmetric(horizontal: 10),
              decoration: BoxDecoration(
                color: option.$2 ? Colors.white : Colors.transparent,
                borderRadius: BorderRadius.circular(5),
              ),
              child: Text(
                option.$1,
                style: const TextStyle(fontSize: 12, color: Color(0xff4c4c4c)),
              ),
            ),
          ),
      ],
    ),
  );

  Widget _propertySlider(
    String label,
    double value,
    double min,
    double max,
    String display,
    ValueChanged<double> onChanged,
  ) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      if (label.isNotEmpty) ...[
        Text(
          label,
          style: const TextStyle(fontSize: 11, color: Color(0xff6b7280)),
        ),
        const SizedBox(width: 6),
      ],
      SizedBox(
        width: 72,
        child: SliderTheme(
          data: SliderTheme.of(context).copyWith(
            trackHeight: 4,
            activeTrackColor: const Color(0xff4c4c4c),
            inactiveTrackColor: const Color(0xffc8cbd0),
            thumbColor: const Color(0xff4c4c4c),
            thumbShape: const RoundSliderThumbShape(enabledThumbRadius: 7),
            overlayShape: SliderComponentShape.noOverlay,
          ),
          child: Slider(value: value, min: min, max: max, onChanged: onChanged),
        ),
      ),
      SizedBox(
        width: 38,
        child: Text(
          display,
          textAlign: TextAlign.right,
          style: const TextStyle(
            fontSize: 12,
            fontWeight: FontWeight.w500,
            color: Color(0xff374151),
          ),
        ),
      ),
    ],
  );

  Widget _actionButton(
    String icon,
    String label,
    VoidCallback? action, {
    bool active = false,
  }) => Tooltip(
    message: label,
    child: Material(
      color: active ? const Color(0x160f172a) : Colors.transparent,
      borderRadius: BorderRadius.circular(9),
      child: InkWell(
        borderRadius: BorderRadius.circular(9),
        hoverColor: icon == 'close'
            ? const Color(0x1ffa5151)
            : icon == 'check'
            ? const Color(0x2100c375)
            : const Color(0x0e0f172a),
        onTap: action,
        child: SizedBox(
          width: 40,
          height: 46,
          child: Center(
            child: ShotIcon(
              icon,
              color: action == null
                  ? const Color(0xffaeaeae)
                  : icon == 'close'
                  ? const Color(0xfffa5151)
                  : icon == 'check'
                  ? const Color(0xff00c375)
                  : const Color(0xff4c4c4c),
            ),
          ),
        ),
      ),
    ),
  );

  Widget _toolButton(String icon, String label, AnnotationTool tool) => Tooltip(
    message: label,
    child: Material(
      color:
          _tool == tool ||
              (tool == AnnotationTool.pen &&
                  _tool == AnnotationTool.highlighter)
          ? const Color(0x160f172a)
          : Colors.transparent,
      borderRadius: BorderRadius.circular(9),
      child: InkWell(
        borderRadius: BorderRadius.circular(9),
        hoverColor: const Color(0x0e0f172a),
        onTap: () => setState(() {
          _tool = _tool == tool ? null : tool;
          _qrMode = false;
          if (tool == AnnotationTool.sticker) unawaited(_loadStickerIcons());
        }),
        child: SizedBox(
          width: 40,
          height: 46,
          child: Center(
            child: ShotIcon(
              icon,
              size: 22,
              color: _tool == tool
                  ? const Color(0xff1f2937)
                  : const Color(0xff4c4c4c),
            ),
          ),
        ),
      ),
    ),
  );

  Offset _withinDisplay(Offset point) => Offset(
    point.dx.clamp(0.0, _display.width),
    point.dy.clamp(0.0, _display.height),
  );

  Widget _selectionHandle(Rect rect, String edge) {
    final horizontal = edge == 't' || edge == 'b';
    final vertical = edge == 'l' || edge == 'r';
    final left = edge.contains('l')
        ? rect.left - 8
        : edge.contains('r')
        ? rect.right - 8
        : rect.left + 8;
    final top = edge.contains('t')
        ? rect.top - 8
        : edge.contains('b')
        ? rect.bottom - 8
        : rect.top + 8;
    final cursor = horizontal
        ? SystemMouseCursors.resizeUpDown
        : vertical
        ? SystemMouseCursors.resizeLeftRight
        : (edge == 'tl' || edge == 'br')
        ? SystemMouseCursors.resizeUpLeftDownRight
        : SystemMouseCursors.resizeUpRightDownLeft;
    return Positioned(
      left: left,
      top: top,
      child: MouseRegion(
        cursor: cursor,
        child: Listener(
          behavior: HitTestBehavior.opaque,
          onPointerDown: (event) {
            if (event.buttons != 1) return;
            _resizeOrigin = _selection;
            _resizePointer = event.position;
            _resizeEdge = edge;
            setState(() => _hoverWindow = null);
          },
          onPointerMove: (event) => _resizeSelection(event.position),
          onPointerUp: (_) {
            _resizeOrigin = null;
            _resizePointer = null;
            _resizeEdge = null;
          },
          child: SizedBox(
            width: horizontal ? math.max(16.0, rect.width - 16) : 16,
            height: vertical ? math.max(16.0, rect.height - 16) : 16,
            child: Center(
              child: Container(
                width: horizontal
                    ? 12
                    : vertical
                    ? 3
                    : 8,
                height: vertical
                    ? 12
                    : horizontal
                    ? 3
                    : 8,
                decoration: BoxDecoration(
                  color: Colors.white,
                  border: Border.all(color: const Color(0xff1688f0)),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _SelectionPainter extends CustomPainter {
  const _SelectionPainter({
    required this.selection,
    required this.annotations,
    required this.draft,
    required this.qrRect,
  });
  final Rect? selection;
  final List<Annotation> annotations;
  final Annotation? draft;
  final Rect? qrRect;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = selection?.intersect(Offset.zero & size);
    final shade = Paint()..color = const Color(0x330f172a);
    if (rect == null || rect.isEmpty) {
      canvas.drawRect(Offset.zero & size, shade);
    } else {
      canvas.drawRect(Rect.fromLTRB(0, 0, size.width, rect.top), shade);
      canvas.drawRect(
        Rect.fromLTRB(0, rect.bottom, size.width, size.height),
        shade,
      );
      canvas.drawRect(
        Rect.fromLTRB(0, rect.top, rect.left, rect.bottom),
        shade,
      );
      canvas.drawRect(
        Rect.fromLTRB(rect.right, rect.top, size.width, rect.bottom),
        shade,
      );
      canvas.drawRect(
        rect,
        Paint()
          ..color = const Color(0xff1688f0)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2,
      );
      canvas.save();
      canvas.clipRect(rect);
      for (final annotation in annotations) {
        annotation.paint(canvas);
      }
      draft?.paint(canvas);
      canvas.restore();
      if (qrRect != null) {
        canvas.drawRect(
          qrRect!,
          Paint()
            ..color = const Color(0xffffaa00)
            ..style = PaintingStyle.stroke
            ..strokeWidth = 2,
        );
      }
    }
  }

  @override
  bool shouldRepaint(covariant _SelectionPainter old) =>
      old.selection != selection ||
      old.annotations.length != annotations.length ||
      old.draft != draft ||
      old.qrRect != qrRect;
}

class _LongBorderPainter extends CustomPainter {
  const _LongBorderPainter(this.selection);
  final Rect? selection;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = selection;
    if (rect == null) return;
    canvas.drawRect(
      rect.deflate(1.5),
      Paint()
        ..color = const Color(0xff1688f0)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 3,
    );
  }

  @override
  bool shouldRepaint(covariant _LongBorderPainter oldDelegate) =>
      oldDelegate.selection != selection;
}

class _TextEditorBorderPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final paint = Paint()
      ..color = const Color(0xb3ffffff)
      ..strokeWidth = 1;
    for (double x = 0; x < size.width; x += 8) {
      final end = math.min(x + 4, size.width);
      canvas.drawLine(Offset(x, 0), Offset(end, 0), paint);
      canvas.drawLine(Offset(x, size.height), Offset(end, size.height), paint);
    }
    for (double y = 0; y < size.height; y += 8) {
      final end = math.min(y + 4, size.height);
      canvas.drawLine(Offset(0, y), Offset(0, end), paint);
      canvas.drawLine(Offset(size.width, y), Offset(size.width, end), paint);
    }
  }

  @override
  bool shouldRepaint(covariant _TextEditorBorderPainter oldDelegate) => false;
}

class _MagnifierPainter extends CustomPainter {
  const _MagnifierPainter({
    required this.image,
    required this.position,
    required this.displaySize,
    required this.color,
  });

  final ui.Image image;
  final Offset position;
  final Size displaySize;
  final String color;

  @override
  void paint(Canvas canvas, Size size) {
    final sourceWidth = 39 * image.width / displaySize.width;
    final sourceHeight = 39 * image.height / displaySize.height;
    final x = (position.dx * image.width / displaySize.width).clamp(
      sourceWidth / 2,
      image.width - sourceWidth / 2,
    );
    final y = (position.dy * image.height / displaySize.height).clamp(
      sourceHeight / 2,
      image.height - sourceHeight / 2,
    );
    final sample = Rect.fromCenter(
      center: Offset(x, y),
      width: sourceWidth,
      height: sourceHeight,
    );
    final bounds = RRect.fromRectAndRadius(
      Offset.zero & size,
      const Radius.circular(7),
    );
    canvas.drawRRect(bounds, Paint()..color = const Color(0xfff8fafc));
    canvas.save();
    canvas.clipRRect(bounds);
    canvas.drawImageRect(
      image,
      sample,
      const Rect.fromLTWH(0, 0, 156, 156),
      Paint()..filterQuality = FilterQuality.high,
    );
    canvas.drawLine(
      const Offset(78, 69),
      const Offset(78, 87),
      Paint()
        ..color = Colors.white
        ..strokeWidth = 1,
    );
    canvas.drawLine(
      const Offset(69, 78),
      const Offset(87, 78),
      Paint()
        ..color = Colors.white
        ..strokeWidth = 1,
    );
    canvas.drawRect(
      const Rect.fromLTWH(0, 156, 156, 24),
      Paint()..color = const Color(0xfff8fafc),
    );
    canvas.drawRect(
      const Rect.fromLTWH(7, 161, 14, 14),
      Paint()
        ..color = Color(int.parse(color.substring(1), radix: 16) | 0xff000000),
    );
    final label = TextPainter(
      text: TextSpan(
        text: color,
        style: const TextStyle(
          color: Color(0xff27313d),
          fontSize: 12,
          fontFamily: 'monospace',
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    label.paint(canvas, const Offset(28, 159));
    final hint = TextPainter(
      text: const TextSpan(
        text: '按 C 复制颜色',
        style: TextStyle(color: Color(0xff8a919f), fontSize: 10),
      ),
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout(maxWidth: 69);
    hint.paint(canvas, const Offset(84, 161));
    canvas.restore();
    canvas.drawRRect(
      bounds,
      Paint()
        ..color = const Color(0xffcbd5e1)
        ..style = PaintingStyle.stroke,
    );
  }

  @override
  bool shouldRepaint(covariant _MagnifierPainter old) =>
      old.image != image ||
      old.position != position ||
      old.displaySize != displaySize ||
      old.color != color;
}
