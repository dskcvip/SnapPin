import 'dart:convert';
import 'dart:io';
import 'dart:ui';

import 'package:desktop_multi_window/desktop_multi_window.dart';
import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

import 'capture_bridge.dart';
import 'app_config.dart';
import 'app_log.dart';
import 'windows/ball_window.dart';
import 'windows/app_menu_window.dart';
import 'windows/capture_overlay.dart';
import 'windows/pin_window.dart';
import 'windows/pin_manager_window.dart';
import 'windows/pin_menu_window.dart';
import 'windows/settings_window.dart';
import 'windows/translation_window.dart';
import 'windows/ocr_window.dart';
import 'windows/long_preview_window.dart';
import 'windows/capture_toast_window.dart';

Future<void> main(List<String> args) async {
  final startup = Stopwatch()..start();
  WidgetsFlutterBinding.ensureInitialized();
  AppLog.initialize();
  AppLog.write('startup-binding', '${startup.elapsedMilliseconds}ms');
  FlutterError.onError = (details) {
    AppLog.write('flutter-error', '${details.exception}\n${details.stack}');
    FlutterError.presentError(details);
  };
  PlatformDispatcher.instance.onError = (error, stack) {
    AppLog.write('unhandled-error', '$error\n$stack');
    return true;
  };
  debugPrint('SnapPin startup: binding ready');
  await windowManager.ensureInitialized();
  AppLog.write('startup-window-manager', '${startup.elapsedMilliseconds}ms');
  debugPrint('SnapPin startup: window manager ready');
  final controller = await WindowController.fromCurrentEngine();
  AppLog.write('startup-window-controller', '${startup.elapsedMilliseconds}ms');
  debugPrint('SnapPin startup: window role ${controller.arguments}');
  final arguments = controller.arguments.isEmpty
      ? <String, dynamic>{'role': 'ball'}
      : Map<String, dynamic>.from(jsonDecode(controller.arguments) as Map);
  AppLog.write(
    'window-start',
    'id=${controller.windowId} role=${arguments['role'] ?? 'ball'}',
  );
  switch (arguments['role']) {
    case 'capture-toast':
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(135, 120),
          center: true,
          alwaysOnTop: true,
          backgroundColor: Colors.transparent,
          skipTaskbar: true,
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async => windowManager.show(),
      );
      runApp(const SnapPinApp(child: CaptureToastWindow()));
    case 'long-preview':
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(344, 560),
          alwaysOnTop: true,
          backgroundColor: Colors.transparent,
          skipTaskbar: true,
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async {
          await windowManager.setPosition(
            Offset(
              (arguments['x'] as num).toDouble(),
              (arguments['y'] as num).toDouble(),
            ),
          );
          await windowManager.show();
        },
      );
      runApp(
        SnapPinApp(
          child: LongPreviewWindow(
            controller: controller,
            initialHeight: arguments['height'] as int,
          ),
        ),
      );
    case 'overlay':
      final display = arguments['display'] is Map
          ? CapturedDisplay.fromMap(
              Map<String, dynamic>.from(arguments['display'] as Map),
            )
          : null;
      final prewarmBounds =
          arguments['prewarmWidth'] is num && arguments['prewarmHeight'] is num
          ? Rect.fromLTWH(
              (arguments['prewarmX'] as num).toDouble(),
              (arguments['prewarmY'] as num).toDouble(),
              (arguments['prewarmWidth'] as num).toDouble(),
              (arguments['prewarmHeight'] as num).toDouble(),
            )
          : null;
      await windowManager.waitUntilReadyToShow(
        WindowOptions(
          size: display == null
              ? prewarmBounds?.size ?? const Size(1, 1)
              : Size(display.width, display.height),
          alwaysOnTop: true,
          backgroundColor: Colors.transparent,
          skipTaskbar: true,
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async {
          if (display != null) {
            await windowManager.setPosition(Offset(display.x, display.y));
            await windowManager.show();
            await windowManager.focus();
          } else if (prewarmBounds != null) {
            await windowManager.setPosition(prewarmBounds.topLeft);
          }
        },
      );
      runApp(
        SnapPinApp(
          child: CaptureOverlay(
            display: display,
            controller: controller,
            prewarmBounds: prewarmBounds,
          ),
        ),
      );
    case 'pin':
      final size = Size(
        (arguments['pixelWidth'] as num? ?? arguments['width'] as num)
            .toDouble(),
        (arguments['pixelHeight'] as num? ?? arguments['height'] as num)
            .toDouble(),
      );
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(1100, 760),
          center: true,
          alwaysOnTop: true,
          backgroundColor: Color(0xfff8f8f8),
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async {
          await windowManager.setAsFrameless();
          await windowManager.setHasShadow(false);
          await windowManager.show();
          await windowManager.focus();
        },
      );
      runApp(
        SnapPinApp(
          child: PinWindow(
            controller: controller,
            path: arguments['path'] as String,
            baseSize: size,
          ),
        ),
      );
    case 'pin-manager':
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(560, 560),
          center: true,
          backgroundColor: Color(0xfff8fafc),
          title: '贴图管理',
        ),
        () async {
          await windowManager.show();
          await windowManager.focus();
        },
      );
      runApp(SnapPinApp(child: PinManagerWindow(controller: controller)));
    case 'settings':
      await windowManager.setPreventClose(true);
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(782, 672),
          center: true,
          backgroundColor: Colors.white,
          title: 'SnapPin 设置',
        ),
        () async {
          await windowManager.show();
          await windowManager.focus();
        },
      );
      runApp(SnapPinApp(child: SettingsWindow(controller: controller)));
    case 'translation':
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(820, 550),
          center: true,
          backgroundColor: Colors.white,
          title: '翻译',
        ),
        () async {
          await windowManager.show();
          await windowManager.focus();
        },
      );
      runApp(
        SnapPinApp(
          child: TranslationWindow(
            controller: controller,
            source: arguments['source'] as String,
          ),
        ),
      );
    case 'ocr':
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(820, 550),
          center: true,
          backgroundColor: Colors.white,
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async {
          await windowManager.setAsFrameless();
          await windowManager.show();
          await windowManager.focus();
        },
      );
      runApp(
        SnapPinApp(
          child: OcrWindow(
            controller: controller,
            imagePath: arguments['image'] as String,
            text: arguments['text'] as String,
          ),
        ),
      );
    case 'pin-menu':
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(650, 620),
          alwaysOnTop: true,
          backgroundColor: Colors.transparent,
          skipTaskbar: true,
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async {
          await windowManager.setAsFrameless();
          await windowManager.setHasShadow(false);
          await windowManager.setPosition(
            Offset(
              (arguments['x'] as num).toDouble(),
              (arguments['y'] as num).toDouble(),
            ),
          );
          await windowManager.show();
          await windowManager.focus();
        },
      );
      runApp(
        SnapPinApp(
          child: PinMenuWindow(
            controller: controller,
            targetId: arguments['target'] as String,
          ),
        ),
      );
    case 'menu':
      await windowManager.waitUntilReadyToShow(
        const WindowOptions(
          size: Size(260, 312),
          alwaysOnTop: true,
          backgroundColor: Colors.transparent,
          skipTaskbar: true,
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async {
          await windowManager.setAsFrameless();
          await windowManager.setHasShadow(false);
          if (arguments['prewarm'] != true) {
            await windowManager.setPosition(
              Offset(
                (arguments['x'] as num).toDouble(),
                (arguments['y'] as num).toDouble(),
              ),
            );
            await windowManager.show();
            await windowManager.focus();
          }
        },
      );
      runApp(SnapPinApp(child: AppMenuWindow(controller: controller)));
    default:
      debugPrint('SnapPin startup: preparing ball');
      final config = await ConfigStore.load();
      AppLog.write('startup-config', '${startup.elapsedMilliseconds}ms');
      await windowManager.waitUntilReadyToShow(
        WindowOptions(
          size: Size.square(config.ballSize + 18),
          alwaysOnTop: true,
          backgroundColor: Colors.transparent,
          skipTaskbar: true,
          titleBarStyle: TitleBarStyle.hidden,
          windowButtonVisibility: false,
        ),
        () async {
          await windowManager.setAsFrameless();
          await windowManager.setHasShadow(false);
        },
      );
      debugPrint('SnapPin startup: running ball');
      AppLog.write('startup-run-app', '${startup.elapsedMilliseconds}ms');
      runApp(
        SnapPinApp(
          child: BallWindow(controller: controller, config: config),
        ),
      );
  }
}

class SnapPinApp extends StatelessWidget {
  const SnapPinApp({super.key, required this.child});
  final Widget child;

  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'SnapPin',
    debugShowCheckedModeBanner: false,
    theme: ThemeData(
      useMaterial3: false,
      colorScheme: const ColorScheme.light(primary: Color(0xff12a5d8)),
      fontFamily: Platform.isWindows ? 'Segoe UI' : null,
      fontFamilyFallback: Platform.isWindows ? const ['Microsoft YaHei'] : null,
    ),
    home: Scaffold(backgroundColor: Colors.transparent, body: child),
  );
}
