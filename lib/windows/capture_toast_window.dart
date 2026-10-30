import 'dart:async';

import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

class CaptureToastWindow extends StatefulWidget {
  const CaptureToastWindow({super.key});

  @override
  State<CaptureToastWindow> createState() => _CaptureToastWindowState();
}

class _CaptureToastWindowState extends State<CaptureToastWindow> {
  Timer? _timer;

  @override
  void initState() {
    super.initState();
    _timer = Timer(const Duration(milliseconds: 1400), windowManager.close);
  }

  @override
  void dispose() {
    _timer?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => Center(
    child: Container(
      width: 135,
      height: 120,
      decoration: BoxDecoration(
        color: const Color(0xff4b4b4b),
        borderRadius: BorderRadius.circular(8),
      ),
      child: const Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          Icon(Icons.check, size: 48, color: Colors.white),
          SizedBox(height: 8),
          Text('已添加到剪贴板', style: TextStyle(fontSize: 13, color: Colors.white)),
        ],
      ),
    ),
  );
}
