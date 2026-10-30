import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;
import 'dart:ui' as ui;
import 'dart:typed_data';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:hotkey_manager/hotkey_manager.dart';
import 'package:pasteboard/pasteboard.dart';
import 'package:screen_retriever/screen_retriever.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:window_manager/window_manager.dart';

import '../capture_bridge.dart';
import 'app_menu_metrics.dart';
import '../app_config.dart';
import '../app_log.dart';
import '../ocr_bridge.dart';
import '../translation_service.dart';
import '../shortcuts.dart';
import '../long_stitch.dart';
import '../long_stitch_worker.dart';

class BallWindow extends StatefulWidget {
  const BallWindow({super.key, required this.controller, required this.config});
  final WindowController controller;
  final AppConfig config;
  @override
  State<BallWindow> createState() => _BallWindowState();
}

class _BallWindowState extends State<BallWindow>
    with TrayListener, WindowListener {
  static const double _ballMargin = 9;
  bool _capturing = false;
  bool _captureActive = false;
  Offset? _down;
  bool _dragged = false;
  bool _hovered = false;
  bool _pressed = false;
  late AppConfig _config;
  Timer? _positionSave;
  bool _movingForConfig = false;
  bool _previewingSettings = false;
  Future<WindowController>? _menuFuture;
  Future<List<WindowController>>? _overlayFuture;
  LongStitchWorker? _longSession;
  Timer? _longTimer;
  bool _longSampling = false;
  bool _longStopping = false;
  bool _longStarting = false;
  Map<String, dynamic>? _longRegion;
  WindowController? _longPreview;
  Directory? _longPreviewDirectory;
  int _longPreviewSequence = 0;
  LongStepKind? _lastLongStepKind;
  int _longStillSamples = 0;
  DateTime? _longNoMatchSince;
  bool _longWarningShown = false;
  DateTime _longLastThumbnail = DateTime.fromMillisecondsSinceEpoch(0);
  final List<String> _hotkeyWarnings = [];
  bool _hotkeysSuspended = false;
  Future<void> _hotkeyOperation = Future<void>.value();

  @override
  void initState() {
    super.initState();
    _config = widget.config;
    CaptureBridge.onLongEscape(_finishLongCapture);
    windowManager.addListener(this);
    widget.controller.setWindowMethodHandler((call) async {
      if (call.method == 'captureFinished') {
        _captureActive = false;
        await windowManager.show();
        debugPrint(
          'SnapPin capture finished: ball visible=${await windowManager.isVisible()}',
        );
      }
      if (call.method == 'longCaptureStart') {
        await _startLongCapture(
          Map<String, dynamic>.from(
            jsonDecode(call.arguments as String) as Map,
          ),
        );
      }
      if (call.method == 'longStop') await _finishLongCapture();
      if (call.method == 'longCancel') await _cancelLongCapture();
      if (call.method == 'menuAction') {
        AppLog.write('menu-dispatch', call.arguments);
        switch (call.arguments as String) {
          case 'capture':
            _capture();
          case 'pin-clipboard':
            _pinClipboard();
          case 'toggle-ball':
            _toggleBall();
          case 'pin-manager':
            _openPinManager();
          case 'settings':
            _openSettings();
          case 'open-log':
            _openLog();
          case 'quit':
            exit(0);
        }
      }
      if (call.method == 'suspendHotkeys') await _setHotkeysSuspended(true);
      if (call.method == 'resumeHotkeys') await _setHotkeysSuspended(false);
      if (call.method == 'applySettings') {
        final config = AppConfig.fromJson(
          Map<String, dynamic>.from(
            jsonDecode(call.arguments as String) as Map,
          ),
        );
        _previewingSettings = false;
        await _applyConfig(config);
      }
      if (call.method == 'previewBall') {
        final values = Map<String, dynamic>.from(
          jsonDecode(call.arguments as String) as Map,
        );
        _previewingSettings = true;
        await _applyConfig(
          _config.copyWith(
            ballSize: (values['size'] as num).toInt(),
            ballOpacity: (values['opacity'] as num).toDouble(),
          ),
        );
      }
      if (call.method == 'getBallCenter') {
        final position = await windowManager.getPosition();
        final size = await windowManager.getSize();
        return {
          'x': position.dx + size.width / 2,
          'y': position.dy + size.height / 2,
        };
      }
      if (call.method == 'recognizeText') {
        return OcrBridge.recognize(call.arguments as Uint8List);
      }
      if (call.method == 'translateText') {
        final config = await ConfigStore.load();
        if (config.translationModels.isEmpty) {
          throw StateError('请先在设置中添加翻译模型');
        }
        return TranslationService.translate(
          call.arguments as String,
          config.translationModels.first,
          config.targetLang,
        );
      }
      return null;
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      unawaited(_finishStartup());
    });
  }

  Future<void> _finishStartup() async {
    try {
      await _placeBall();
      await Future.wait([_registerHotkey(), _initializeTray()]);
      if (!mounted) return;
      await windowManager.show();
      AppLog.write('ball-ready');
      unawaited(_prewarmOverlays());
    } catch (error, stack) {
      AppLog.write('ball-startup-failed', '$error\n$stack');
      if (mounted) await windowManager.show();
    }
  }

  Future<void> _queueHotkeyOperation(Future<void> Function() operation) {
    final next = _hotkeyOperation
        .catchError((Object error) {
          AppLog.write('hotkey-operation-failed', error);
        })
        .then((_) => operation());
    _hotkeyOperation = next;
    return next;
  }

  Future<void> _setHotkeysSuspended(bool suspended) =>
      _queueHotkeyOperation(() async {
        _hotkeysSuspended = suspended;
        if (suspended) {
          await hotKeyManager.unregisterAll();
          AppLog.write('hotkeys-suspended', 'settings-recording');
        } else {
          await _registerHotkeyNow();
          AppLog.write('hotkeys-resumed', 'settings-recording');
        }
      });

  Future<void> _registerHotkey() => _queueHotkeyOperation(_registerHotkeyNow);

  Future<void> _registerHotkeyNow() async {
    if (_hotkeysSuspended) return;
    try {
      _hotkeyWarnings.clear();
      final capture = parseShortcut(_config.shortcutCapture);
      final pin = parseShortcut(_config.shortcutPinClipboard);
      await hotKeyManager.unregisterAll();
      try {
        await hotKeyManager.register(
          capture,
          keyDownHandler: (_) {
            if (_hotkeysSuspended) return;
            AppLog.write('hotkey-trigger', 'capture');
            _capture();
          },
        );
      } catch (error) {
        AppLog.write('hotkey-registration-failed', 'capture: $error');
        _hotkeyWarnings.add('截图快捷键');
      }
      try {
        await hotKeyManager.register(
          pin,
          keyDownHandler: (_) {
            if (_hotkeysSuspended) return;
            AppLog.write('hotkey-trigger', 'pin-clipboard');
            _pinClipboard();
          },
        );
      } catch (error) {
        AppLog.write('hotkey-registration-failed', 'pin-clipboard: $error');
        _hotkeyWarnings.add('贴图快捷键');
      }
      AppLog.write(
        'hotkey-registration-complete',
        'capture=${_config.shortcutCapture} pin=${_config.shortcutPinClipboard} failed=${_hotkeyWarnings.join(',')}',
      );
    } catch (error) {
      AppLog.write('hotkey-registration-failed', error);
      _hotkeyWarnings.add('快捷键配置无效，请在设置中检查');
    }
  }

  Future<void> _initializeTray() async {
    try {
      trayManager.addListener(this);
      final icon = Platform.isWindows
          ? '${File(Platform.resolvedExecutable).parent.path}${Platform.pathSeparator}data${Platform.pathSeparator}flutter_assets${Platform.pathSeparator}assets${Platform.pathSeparator}tray_icon.ico'
          : 'assets/tray_icon.png';
      AppLog.write('tray-icon', icon);
      await trayManager.setIcon(icon);
      await trayManager.setToolTip('SnapPin');
      AppLog.write('tray-ready');
    } catch (error) {
      AppLog.write('tray-init-failed', error);
    }
  }

  @override
  void dispose() {
    _positionSave?.cancel();
    _longTimer?.cancel();
    _longSession?.dispose();
    windowManager.removeListener(this);
    trayManager.removeListener(this);
    super.dispose();
  }

  @override
  void onWindowMove() {
    if (_movingForConfig) return;
    _positionSave?.cancel();
    _positionSave = Timer(const Duration(milliseconds: 250), () async {
      final position = await windowManager.getPosition();
      final size = await windowManager.getSize();
      final center = position + Offset(size.width / 2, size.height / 2);
      _config = _config.copyWith(ballCenter: center);
      final persisted = _previewingSettings
          ? await ConfigStore.load()
          : _config;
      await ConfigStore.save(persisted.copyWith(ballCenter: center));
    });
  }

  Future<void> _applyConfig(AppConfig config) async {
    _positionSave?.cancel();
    _movingForConfig = true;
    final shortcutsChanged =
        config.shortcutCapture != _config.shortcutCapture ||
        config.shortcutPinClipboard != _config.shortcutPinClipboard;
    final oldPosition = await windowManager.getPosition();
    final oldSize = await windowManager.getSize();
    final center = oldPosition + Offset(oldSize.width / 2, oldSize.height / 2);
    await windowManager.setSize(Size.square(config.ballSize + _ballMargin * 2));
    await windowManager.setPosition(
      center -
          Offset(
            config.ballSize / 2 + _ballMargin,
            config.ballSize / 2 + _ballMargin,
          ),
    );
    if (mounted) setState(() => _config = config.copyWith(ballCenter: center));
    if (shortcutsChanged) await _registerHotkey();
    await Future<void>.delayed(const Duration(milliseconds: 300));
    _movingForConfig = false;
  }

  @override
  void onTrayIconMouseDown() => _openTrayMenu('left');

  @override
  void onTrayIconRightMouseDown() => _openTrayMenu('right');

  Future<void> _openTrayMenu(String button) async {
    AppLog.write('tray-click', button);
    try {
      final cursor = await screenRetriever.getCursorScreenPoint();
      await _openMenu(cursor: cursor);
      AppLog.write('tray-menu-opened', button);
    } catch (error, stack) {
      AppLog.write('tray-menu-failed', '$error\n$stack');
    }
  }

  Future<void> _toggleBall() async {
    if (await windowManager.isVisible()) {
      await windowManager.hide();
    } else {
      await windowManager.show();
    }
  }

  Future<void> _openLog() async {
    if (AppLog.path.isEmpty) return;
    AppLog.write('open-log');
    await Process.start(Platform.isMacOS ? 'open' : 'explorer.exe', [
      AppLog.path,
    ]);
  }

  Future<void> _openPinManager() async {
    for (final window in await WindowController.getAll()) {
      if (window.arguments.isEmpty) continue;
      if ((jsonDecode(window.arguments) as Map)['role'] == 'pin-manager') {
        await window.invokeMethod<void>('focusManager');
        return;
      }
    }
    await WindowController.create(
      WindowConfiguration(
        arguments: jsonEncode({'role': 'pin-manager'}),
        hiddenAtLaunch: true,
      ),
    );
  }

  Future<void> _openSettings() async {
    for (final window in await WindowController.getAll()) {
      if (window.arguments.isEmpty) continue;
      if ((jsonDecode(window.arguments) as Map)['role'] == 'settings') {
        await window.invokeMethod<void>('focusSettings');
        return;
      }
    }
    await WindowController.create(
      WindowConfiguration(
        arguments: jsonEncode({'role': 'settings'}),
        hiddenAtLaunch: true,
      ),
    );
  }

  Future<void> _pinClipboard() async {
    AppLog.write('pin-clipboard-start');
    try {
      final bytes = await Pasteboard.image;
      if (bytes == null || bytes.isEmpty) throw StateError('剪贴板中没有图像');
      final codec = await ui.instantiateImageCodec(bytes);
      final frame = await codec.getNextFrame();
      final width = frame.image.width.toDouble();
      final height = frame.image.height.toDouble();
      frame.image.dispose();
      codec.dispose();
      final display = await screenRetriever.getPrimaryDisplay();
      final scaleFactor = display.scaleFactor ?? 1.0;
      final logicalWidth = width / scaleFactor;
      final logicalHeight = height / scaleFactor;
      final scale = math.min(
        1.0,
        math.min(900 / logicalWidth, 700 / logicalHeight),
      );
      final directory = await Directory.systemTemp.createTemp('snappin-pin-');
      final path = '${directory.path}${Platform.pathSeparator}pin.png';
      await File(path).writeAsBytes(bytes);
      await WindowController.create(
        WindowConfiguration(
          arguments: jsonEncode({
            'role': 'pin',
            'path': path,
            'width': logicalWidth * scale + 20,
            'height': logicalHeight * scale + 20,
            'pixelWidth': width,
            'pixelHeight': height,
          }),
          hiddenAtLaunch: true,
        ),
      );
      AppLog.write(
        'pin-clipboard-created',
        '${width.round()}x${height.round()}',
      );
    } catch (error) {
      AppLog.write('pin-clipboard-failed', error);
      final visible = await windowManager.isVisible();
      if (mounted && visible) {
        ScaffoldMessenger.of(context)
            .showSnackBar(SnackBar(content: Text('剪贴板贴图失败：$error')));
      }
    }
  }

  Future<void> _placeBall() async {
    final saved = _config.ballCenter;
    if (saved != null) {
      await windowManager.setPosition(
        saved -
            Offset(
              _config.ballSize / 2 + _ballMargin,
              _config.ballSize / 2 + _ballMargin,
            ),
      );
      return;
    }
    final display = await screenRetriever.getPrimaryDisplay();
    final origin = display.visiblePosition ?? Offset.zero;
    final size = display.visibleSize ?? display.size;
    await windowManager.setPosition(
      Offset(
        origin.dx + 18 - _ballMargin,
        origin.dy + size.height - _config.ballSize - 18 - _ballMargin,
      ),
    );
  }

  Future<void> _capture() async {
    if (_capturing || _captureActive) {
      AppLog.write(
        'capture-skipped',
        'capturing=$_capturing active=$_captureActive',
      );
      return;
    }
    final started = DateTime.now();
    AppLog.write('capture-start');
    setState(() => _capturing = true);
    try {
      await windowManager.hide();
      await Future.delayed(
        Duration(milliseconds: Platform.isWindows ? 80 : 40),
      );
      AppLog.write(
        'capture-ball-hidden',
        '${DateTime.now().difference(started).inMilliseconds}ms',
      );
      final displays = await CaptureBridge.captureDisplays();
      AppLog.write(
        'capture-displays-ready',
        '${displays.length} displays, ${DateTime.now().difference(started).inMilliseconds}ms',
      );
      final cursor = await screenRetriever.getCursorScreenPoint();
      AppLog.write(
        'capture-cursor-ready',
        '${DateTime.now().difference(started).inMilliseconds}ms',
      );
      final windows = await (_overlayFuture ??= _createOverlayPool());
      while (windows.length < displays.length) {
        windows.add(await _newOverlayWindow());
      }
      await Future.wait(
        List.generate(displays.length, (i) async {
          final display = displays[i];
          var window = windows[i];
          try {
            await _startOverlay(window, display, cursor);
          } catch (error) {
            AppLog.write('overlay-recreate', 'index=$i error=$error');
            window = await _newOverlayWindow();
            windows[i] = window;
            await _startOverlay(window, display, cursor);
          }
        }),
      );
      _captureActive = true;
      AppLog.write(
        'capture-overlays-created',
        '${DateTime.now().difference(started).inMilliseconds}ms',
      );
    } catch (error) {
      AppLog.write('capture-failed', error);
      _captureActive = false;
      await windowManager.show();
      if (!mounted) return;
      ScaffoldMessenger.of(context)
          .showSnackBar(SnackBar(content: Text('截图失败：$error')));
    } finally {
      if (mounted) setState(() => _capturing = false);
    }
  }

  Future<WindowController> _newOverlayWindow({Display? display}) =>
      WindowController.create(
        WindowConfiguration(
          arguments: jsonEncode({
            'role': 'overlay',
            'prewarm': true,
            if (display != null) ...{
              'prewarmX': (display.visiblePosition ?? Offset.zero).dx,
              'prewarmY': (display.visiblePosition ?? Offset.zero).dy,
              'prewarmWidth': display.size.width,
              'prewarmHeight': display.size.height,
            },
          }),
          hiddenAtLaunch: true,
        ),
      );

  Future<List<WindowController>> _createOverlayPool() async {
    final started = DateTime.now();
    final displays = await screenRetriever.getAllDisplays();
    final windows = await Future.wait(
      List.generate(
        displays.length,
        (i) => _newOverlayWindow(display: displays[i]),
      ),
    );
    AppLog.write(
      'overlays-prewarmed',
      '${windows.length} windows, ${DateTime.now().difference(started).inMilliseconds}ms',
    );
    return windows;
  }

  Future<void> _prewarmOverlays() async {
    try {
      await (_overlayFuture ??= _createOverlayPool());
    } catch (error) {
      AppLog.write('overlays-prewarm-failed', error);
      _overlayFuture = null;
    }
  }

  Future<void> _startOverlay(
    WindowController window,
    CapturedDisplay display,
    Offset cursor,
  ) async {
    final started = DateTime.now();
    for (var attempt = 0; attempt < 20; attempt++) {
      try {
        await window.invokeMethod<void>('startCapture', {
          ...display.toMap(),
          'cursorX': cursor.dx,
          'cursorY': cursor.dy,
        });
        AppLog.write(
          'overlay-started',
          '${display.pixelWidth}x${display.pixelHeight} '
              '${DateTime.now().difference(started).inMilliseconds}ms',
        );
        return;
      } catch (_) {
        if (attempt == 19) rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    }
  }

  Future<void> _startLongCapture(Map<String, dynamic> region) async {
    if (_longStarting || _longStopping || _longSession != null) {
      throw StateError('长截图正在进行');
    }
    final started = DateTime.now();
    final x = region['x'] as int;
    final y = region['y'] as int;
    final width = region['width'] as int;
    final height = region['height'] as int;
    if (width < 20 || height < 80) throw StateError('长截图选区太小');
    _longStarting = true;
    try {
      final hiddenToolbars = await CaptureBridge.suppressInputToolbar(true);
      AppLog.write('long-input-toolbar-hidden', 'count=$hiddenToolbars');
      final first = await CaptureBridge.captureRegion(x, y, width, height);
      _longSession = await LongStitchWorker.start(first);
      AppLog.write(
        'long-first-frame',
        'requested=${width}x$height actual=${_longSession!.width}x${_longSession!.height} bytes=${first.length}',
      );
      _lastLongStepKind = null;
      _longStillSamples = 0;
      _longNoMatchSince = null;
      _longWarningShown = false;
      _longLastThumbnail = DateTime.fromMillisecondsSinceEpoch(0);
      _longPreviewDirectory = await Directory.systemTemp.createTemp(
        'snappin-long-preview-',
      );
      _longRegion = region;
      final logicalX = (region['logicalX'] as num).toDouble();
      final logicalY = (region['logicalY'] as num).toDouble();
      final logicalWidth = (region['logicalWidth'] as num).toDouble();
      final displays = await screenRetriever.getAllDisplays();
      final display =
          displays.where((item) {
            final origin = item.visiblePosition ?? Offset.zero;
            return (origin & (item.visibleSize ?? item.size)).contains(
              Offset(logicalX, logicalY),
            );
          }).firstOrNull ??
          await screenRetriever.getPrimaryDisplay();
      final bounds = display.visiblePosition ?? Offset.zero;
      final size = display.visibleSize ?? display.size;
      final previewX = (logicalX + logicalWidth + 12).clamp(
        bounds.dx,
        bounds.dx + size.width - 344,
      );
      final previewY = logicalY.clamp(bounds.dy, bounds.dy + size.height - 560);
      if (_longPreview == null) {
        _longPreview = await WindowController.create(
          WindowConfiguration(
            arguments: jsonEncode({
              'role': 'long-preview',
              'x': previewX,
              'y': previewY,
              'height': _longSession!.height,
            }),
            hiddenAtLaunch: true,
          ),
        );
        AppLog.write('long-preview-created');
      } else {
        await _longPreview!.invokeMethod<void>('openPreview', {
          'x': previewX,
          'y': previewY,
          'height': _longSession!.height,
        });
        AppLog.write('long-preview-reused');
      }
      await _sendLongProgress({
        'height': _longSession!.height,
        'steps': 0,
        'warning': false,
        'thumbPath': await _writeLongThumb(),
      });
      AppLog.write('long-preview-initialized');
      await CaptureBridge.setLongEscapeEnabled(true);
      AppLog.write('long-escape-registered');
      _longTimer = Timer.periodic(
        const Duration(milliseconds: 80),
        (_) => _sampleLongCapture(),
      );
      AppLog.write(
        'long-manual-started',
        '${width}x$height ${DateTime.now().difference(started).inMilliseconds}ms; waiting for user scroll',
      );
    } catch (error, stack) {
      AppLog.write('long-start-failed', '$error\n$stack');
      await _cancelLongCapture();
      rethrow;
    } finally {
      _longStarting = false;
    }
  }

  Future<void> _sampleLongCapture() async {
    if (_longSampling ||
        _longStopping ||
        _longSession == null ||
        _longRegion == null)
      return;
    _longSampling = true;
    var previewHidden = false;
    final sampledAt = DateTime.now();
    try {
      final region = _longRegion!;
      previewHidden =
          await _longPreview?.invokeMethod<bool>('prepareCapture') ?? false;
      final captureStarted = DateTime.now();
      final frame = await CaptureBridge.captureRegion(
        region['x'] as int,
        region['y'] as int,
        region['width'] as int,
        region['height'] as int,
      );
      if (_longStopping || _longSession == null) return;
      final captureMs = DateTime.now()
          .difference(captureStarted)
          .inMilliseconds;
      final stitchStarted = DateTime.now();
      final step = await _longSession!.push(frame);
      final stitchMs = DateTime.now().difference(stitchStarted).inMilliseconds;
      if (step.kind == LongStepKind.still) {
        _longStillSamples++;
        if (_longStillSamples % 20 == 0) {
          AppLog.write(
            'long-manual-still',
            'samples=$_longStillSamples height=${step.height} steps=${step.steps} '
                'capture=${captureMs}ms stitch=${stitchMs}ms',
          );
        }
      } else {
        _longStillSamples = 0;
        AppLog.write(
          'long-manual-frame',
          'kind=${step.kind.name} appended=${step.appended} '
              'height=${step.height} width=${_longSession!.width} '
              'direction=${step.direction} previewScale=${_longSession!.previewScale.toStringAsFixed(4)} '
              'canvasBytes=${_longSession!.width * step.height * 4} steps=${step.steps} '
              'score=${step.score.toStringAsFixed(3)} diff=${step.diff.toStringAsFixed(2)} '
              'bytes=${frame.length} capture=${captureMs}ms stitch=${stitchMs}ms '
              'total=${DateTime.now().difference(sampledAt).inMilliseconds}ms',
        );
      }
      if (step.kind == LongStepKind.noMatch) {
        _longNoMatchSince ??= DateTime.now();
      } else if (step.kind == LongStepKind.appended ||
          step.kind == LongStepKind.rewound) {
        _longNoMatchSince = null;
        _longWarningShown = false;
      }
      final recovering = _longNoMatchSince != null;
      final showWarning =
          recovering &&
          !_longWarningShown &&
          DateTime.now().difference(_longNoMatchSince!).inMilliseconds >= 600;
      if (showWarning) {
        _longWarningShown = true;
        AppLog.write(
          'long-overlap-warning',
          'height=${step.height} steps=${step.steps} '
              'since=${_longNoMatchSince!.toIso8601String()}',
        );
      }
      if (step.kind == LongStepKind.appended ||
          step.kind == LongStepKind.rewound ||
          showWarning) {
        if (step.kind != _lastLongStepKind ||
            step.kind == LongStepKind.appended) {
          AppLog.write(
            'long-progress',
            '${step.kind.name} height=${step.height} steps=${step.steps}',
          );
        }
        _lastLongStepKind = step.kind;
        if (_longPreview != null) {
          String? thumbPath;
          final now = DateTime.now();
          if (step.kind == LongStepKind.appended &&
              now.difference(_longLastThumbnail).inMilliseconds >= 300) {
            thumbPath = await _writeLongThumb();
            _longLastThumbnail = now;
          }
          await _sendLongProgress({
            'height': step.height,
            'steps': step.steps,
            'warning': showWarning,
            'message': showWarning
                ? '滚动过快，找不到重叠；请向上滚回已截区域'
                : step.kind == LongStepKind.rewound
                ? '当前内容已截取；继续滚动可扩展长图'
                : '滚动页面继续截图 · 已 ${step.height} px',
            'thumbPath': thumbPath,
          });
        }
      }
    } on LongCaptureLimitException catch (error) {
      AppLog.write(
        'long-memory-limit',
        '$error output=${_longSession?.width}x${_longSession?.height}',
      );
      unawaited(_finishLongCapture(reason: 'memory-limit'));
    } catch (error, stack) {
      AppLog.write('long-sample-failed', '$error\n$stack');
      unawaited(_finishLongCapture(reason: 'capture-error'));
    } finally {
      if (previewHidden) {
        try {
          await _longPreview?.invokeMethod<void>('finishCapture');
        } catch (_) {}
      }
      _longSampling = false;
    }
  }

  Future<void> _sendLongProgress(Map<String, dynamic> value) async {
    final preview = _longPreview;
    if (preview == null) return;
    for (var attempt = 0; attempt < 20; attempt++) {
      try {
        await preview.invokeMethod<void>('longProgress', value);
        return;
      } catch (_) {
        if (attempt == 19) rethrow;
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
    }
  }

  Future<String> _writeLongThumb() async {
    final directory = _longPreviewDirectory;
    final session = _longSession;
    if (directory == null || session == null) throw StateError('长截图预览不可用');
    final path =
        '${directory.path}${Platform.pathSeparator}thumb-${_longPreviewSequence++}.png';
    await File(path).writeAsBytes(await session.previewPng());
    return path;
  }

  Future<void> _finishLongCapture({String reason = 'user-stop'}) async {
    if (_longSession == null || _longStopping) return;
    AppLog.write(
      'long-finish-begin',
      'reason=$reason height=${_longSession!.height} steps=${_longSession!.steps}',
    );
    _longStopping = true;
    _longTimer?.cancel();
    _longTimer = null;
    while (_longSampling) {
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    var saved = false;
    try {
      final session = _longSession!;
      await _sendLongProgress({
        'height': session.height,
        'steps': session.steps,
        'warning': false,
        'finishing': true,
        'message': '正在生成长截图…',
      });
      final directory = await Directory.systemTemp.createTemp('snappin-long-');
      final path = '${directory.path}${Platform.pathSeparator}long.png';
      await session.savePng(File(path));
      saved = true;
      AppLog.write('long-result-saved', path);
      final width = session.width;
      final height = session.height;
      final steps = session.steps;
      await _closeLongPreview();
      AppLog.write('long-preview-closed');
      unawaited(_cleanupLongThumbnails());
      await _closeCaptureOverlays();
      AppLog.write('long-overlays-closed');
      await WindowController.create(
        WindowConfiguration(
          arguments: jsonEncode({
            'role': 'pin',
            'path': path,
            'width': width,
            'height': height,
            'pixelWidth': width,
            'pixelHeight': height,
          }),
          hiddenAtLaunch: true,
        ),
      );
      AppLog.write(
        'long-result-created',
        'viewer=pin ${width}x$height steps=$steps',
      );
      AppLog.write('long-finished', '${width}x$height path=$path');
    } catch (error, stack) {
      AppLog.write('long-finish-failed', '$error\n$stack');
      if (!saved) {
        try {
          await _sendLongProgress({
            'height': _longSession!.height,
            'steps': _longSession!.steps,
            'warning': true,
            'finishing': false,
            'message': '生成失败，已保留截图内容；请再次点击完成重试',
          });
        } catch (previewError) {
          AppLog.write('long-finish-feedback-failed', previewError);
        }
      }
    } finally {
      if (saved) {
        _longSession?.dispose();
        _longSession = null;
        _lastLongStepKind = null;
        _longRegion = null;
        await _unregisterLongEscape();
        await _restoreLongInputToolbar();
      }
      _longStopping = false;
    }
  }

  Future<void> _cancelLongCapture() async {
    if (_longStopping) return;
    _longTimer?.cancel();
    _longTimer = null;
    _longStopping = true;
    while (_longSampling) {
      await Future<void>.delayed(const Duration(milliseconds: 20));
    }
    _longSession?.dispose();
    _longSession = null;
    _lastLongStepKind = null;
    _longRegion = null;
    try {
      await _unregisterLongEscape();
      await _restoreLongInputToolbar();
      await _closeLongPreview();
      unawaited(_cleanupLongThumbnails());
      await _closeCaptureOverlays();
      AppLog.write('long-cancelled');
    } finally {
      _longStopping = false;
    }
  }

  Future<void> _unregisterLongEscape() async {
    try {
      await CaptureBridge.setLongEscapeEnabled(false);
    } catch (error) {
      AppLog.write('long-escape-unregister-failed', error);
    }
  }

  Future<void> _restoreLongInputToolbar() async {
    try {
      await CaptureBridge.suppressInputToolbar(false);
      AppLog.write('long-input-toolbar-restored');
    } catch (error) {
      AppLog.write('long-input-toolbar-restore-failed', error);
    }
  }

  Future<void> _closeLongPreview() async {
    final preview = _longPreview;
    if (preview != null) {
      try {
        await preview.invokeMethod<void>('closePreview');
      } catch (_) {}
    }
  }

  Future<void> _cleanupLongThumbnails() async {
    final directory = _longPreviewDirectory;
    _longPreviewDirectory = null;
    if (directory == null) return;
    await Future<void>.delayed(const Duration(seconds: 2));
    try {
      await directory.delete(recursive: true);
    } catch (_) {}
  }

  Future<void> _closeCaptureOverlays() async {
    final overlays = await (_overlayFuture ??= _createOverlayPool());
    for (final overlay in overlays) {
      try {
        await overlay.invokeMethod<void>('closeSelf');
      } catch (_) {}
    }
    _captureActive = false;
    await windowManager.show();
  }

  @override
  Widget build(BuildContext context) => MouseRegion(
    cursor: SystemMouseCursors.click,
    onEnter: (_) => setState(() => _hovered = true),
    onExit: (_) => setState(() {
      _hovered = false;
      _pressed = false;
    }),
    child: SizedBox.expand(
      child: Listener(
        onPointerDown: (event) {
          if (event.buttons == 2) {
            AppLog.write('ball-right-click');
            _openMenu();
            return;
          }
          if (event.buttons != 1) return;
          _down = event.position;
          _dragged = false;
          setState(() => _pressed = true);
        },
        onPointerMove: (event) {
          if (_down == null || _dragged) return;
          if ((event.position - _down!).distance > 4) {
            _dragged = true;
            windowManager.startDragging();
          }
        },
        onPointerUp: (_) {
          setState(() => _pressed = false);
          if (_down != null && !_dragged) {
            AppLog.write('ball-left-click');
            _capture();
          }
          _down = null;
        },
        child: Center(
          child: AnimatedScale(
            scale: _pressed
                ? .95
                : _hovered
                ? 1.06
                : 1,
            duration: const Duration(milliseconds: 120),
            curve: Curves.easeOut,
            child: Opacity(
              opacity: _config.ballOpacity,
              child: AnimatedContainer(
                duration: const Duration(milliseconds: 120),
                curve: Curves.easeOut,
                width: _config.ballSize.toDouble(),
                height: _config.ballSize.toDouble(),
                foregroundDecoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(_config.ballSize * .3),
                  border: Border.all(
                    color: _hovered
                        ? const Color(0x4dffffff)
                        : const Color(0x38ffffff),
                  ),
                ),
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(_config.ballSize * .3),
                  boxShadow: _hovered
                      ? const [
                          BoxShadow(color: Color(0x2effffff), spreadRadius: 3),
                          BoxShadow(
                            color: Color(0x42020a18),
                            blurRadius: 5,
                            offset: Offset(0, 2),
                          ),
                        ]
                      : null,
                  gradient: LinearGradient(
                    begin: Alignment.topCenter,
                    end: Alignment.bottomCenter,
                    stops: const [0, .58, 1],
                    colors: [
                      _lightenBallColor(_config.color),
                      _config.color,
                      _config.color,
                    ],
                  ),
                ),
                child: CustomPaint(painter: _BallGlyphPainter()),
              ),
            ),
          ),
        ),
      ),
    ),
  );

  Future<WindowController> _createMenuWindow() async {
    final started = DateTime.now();
    final window = await WindowController.create(
      WindowConfiguration(
        arguments: jsonEncode({'role': 'menu', 'prewarm': true}),
        hiddenAtLaunch: true,
      ),
    );
    AppLog.write(
      'menu-prewarmed',
      '${DateTime.now().difference(started).inMilliseconds}ms',
    );
    return window;
  }

  Future<void> _openMenu({Offset? cursor}) async {
    final started = DateTime.now();
    AppLog.write('menu-open-start', cursor == null ? 'ball' : 'tray');
    try {
      final anchor = cursor ?? await windowManager.getPosition();
      final displays = await screenRetriever.getAllDisplays();
      final display =
          displays.where((candidate) {
            final origin = candidate.visiblePosition ?? Offset.zero;
            final size = candidate.visibleSize ?? candidate.size;
            return (origin & size).contains(anchor);
          }).firstOrNull ??
          await screenRetriever.getPrimaryDisplay();
      final origin = display.visiblePosition ?? Offset.zero;
      final size = display.visibleSize ?? display.size;
      final left = (anchor.dx - (cursor == null ? 218 : 18))
          .clamp(origin.dx, origin.dx + size.width - 260)
          .toDouble();
      final menuHeight = appMenuHeight(_hotkeyWarnings);
      final top = anchor.dy
          .clamp(origin.dy, origin.dy + size.height - menuHeight)
          .toDouble();
      final menu = await (_menuFuture ??= _createMenuWindow());
      for (var attempt = 0; attempt < 20; attempt++) {
        try {
          await menu.invokeMethod<void>('openAt', {
            'x': left,
            'y': top,
            'captureShortcut': _config.shortcutCapture,
            'pinShortcut': _config.shortcutPinClipboard,
            'warnings': List<String>.of(_hotkeyWarnings),
          });
          AppLog.write(
            'menu-opened',
            '${DateTime.now().difference(started).inMilliseconds}ms',
          );
          return;
        } catch (error) {
          if (attempt == 19) rethrow;
          await Future<void>.delayed(const Duration(milliseconds: 50));
        }
      }
    } catch (error, stack) {
      _menuFuture = null;
      AppLog.write('menu-open-failed', '$error\n$stack');
    }
  }
}

Color _lightenBallColor(Color color) {
  final argb = color.toARGB32();
  return Color.fromARGB(
    (argb >> 24) & 0xff,
    math.min(255, ((argb >> 16) & 0xff) + 55),
    math.min(255, ((argb >> 8) & 0xff) + 55),
    math.min(255, (argb & 0xff) + 55),
  );
}

class _BallGlyphPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final glyphSize = size.width * .64;
    canvas.save();
    canvas.translate(
      (size.width - glyphSize) / 2,
      (size.height - glyphSize) / 2,
    );
    canvas.scale(glyphSize / 1024);
    final path = Path()
      ..moveTo(700, 300)
      ..cubicTo(560, 210, 420, 215, 375, 330)
      ..cubicTo(340, 430, 480, 470, 600, 500)
      ..cubicTo(680, 520, 690, 590, 648, 655)
      ..cubicTo(600, 730, 470, 745, 360, 700);
    canvas.drawPath(
      path,
      Paint()
        ..color = Colors.white
        ..style = PaintingStyle.stroke
        ..strokeWidth = 104
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round,
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}
