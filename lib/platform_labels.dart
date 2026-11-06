import 'package:flutter/foundation.dart';

String platformShortcut(String shortcut) =>
    defaultTargetPlatform == TargetPlatform.macOS
    ? shortcut.replaceAll('Ctrl+', '⌘+').replaceAll('Alt+', '⌥+')
    : shortcut;
