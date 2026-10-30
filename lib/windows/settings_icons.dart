import 'package:flutter/material.dart';
import 'package:flutter_svg/flutter_svg.dart';

// SVG paths shared with the Tauri settings reference.
const _icons = <String, String>{
  'search': '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>',
  'general': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1.03 1.56V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1.11-1.56 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.56-1.03H3a2 2 0 1 1 0-4h.09A1.7 1.7 0 0 0 4.65 8.86a1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.7 1.7 0 0 0 9 4.6a1.7 1.7 0 0 0 1.03-1.56V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1.03 1.56 1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v0a1.7 1.7 0 0 0 1.56 1.03H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.51 1.03z" />
    </svg>''',
  'keys': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8" />
    </svg>''',
  'model': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <path d="M12 2 3 7v10l9 5 9-5V7z" />
      <path d="M3 7l9 5 9-5M12 22V12" />
    </svg>''',
  'data': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <ellipse cx="12" cy="5" rx="8" ry="3" />
      <path d="M4 5v7c0 1.7 3.6 3 8 3s8-1.3 8-3V5M4 12v7c0 1.7 3.6 3 8 3s8-1.3 8-3v-7" />
    </svg>''',
  'pencil': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <path d="M17 3a2.8 2.8 0 0 1 4 4L7.5 20.5 2 22l1.5-5.5z" />
    </svg>''',
  'link': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" />
      <path d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7" />
    </svg>''',
  'about': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 11v6M12 7h.01" />
    </svg>''',
  'trash': r'''<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8">
      <path d="M3 6h18M8 6V4h8v2M19 6l-1 14H6L5 6M10 11v6M14 11v6" />
    </svg>''',
};

Widget settingsIcon(
  String name, {
  Color color = const Color(0xff8a919f),
  double size = 16,
}) => SvgPicture.string(
  _icons[name]!,
  width: size,
  height: size,
  theme: SvgTheme(currentColor: color),
);
