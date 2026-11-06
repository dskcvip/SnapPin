import 'package:flutter/material.dart';

/// Shared desktop menu surface. Keep a light shadow in the transparent window
/// margin rather than stacking a second native shadow around the whole HWND.
const desktopMenuRadius = 6.0;
const desktopMenuRowHeight = 32.0;
const desktopMenuTextStyle = TextStyle(fontSize: 14, color: Color(0xff1f1f1f));
const desktopMenuHover = Color(0xffebebeb);
final desktopMenuDecoration = BoxDecoration(
  color: Colors.white,
  border: Border.all(color: const Color(0xffdedede)),
  borderRadius: BorderRadius.circular(desktopMenuRadius),
  boxShadow: const [
    BoxShadow(color: Color(0x22000000), blurRadius: 10, offset: Offset(0, 3)),
    BoxShadow(color: Color(0x10000000), blurRadius: 2, offset: Offset(0, 1)),
  ],
);
