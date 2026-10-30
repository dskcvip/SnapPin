import 'dart:convert';
import 'dart:io';

import 'package:flutter/material.dart';

import '../app_config.dart';

class CaptureExitPreference {
  CaptureExitPreference([File? file])
    : file = file ?? File('${ConfigStore.file.parent.path}/capture-exit.json');

  final File file;

  Future<bool> shouldAsk() async {
    try {
      return (jsonDecode(await file.readAsString()) as Map)['ask'] != false;
    } catch (_) {
      return true;
    }
  }

  Future<void> stopAsking() async {
    await file.parent.create(recursive: true);
    await file.writeAsString(jsonEncode({'ask': false}), flush: true);
  }
}

class CaptureExitDecision {
  const CaptureExitDecision({required this.exit, this.dontAskAgain = false});
  final bool exit;
  final bool dontAskAgain;
}

class CaptureExitDialog extends StatefulWidget {
  const CaptureExitDialog({super.key});

  @override
  State<CaptureExitDialog> createState() => _CaptureExitDialogState();
}

class _CaptureExitDialogState extends State<CaptureExitDialog> {
  bool _dontAsk = false;

  void _answer(bool exit) =>
      Navigator.of(context)
          .pop(CaptureExitDecision(exit: exit, dontAskAgain: exit && _dontAsk));

  @override
  Widget build(BuildContext context) => Dialog(
    backgroundColor: Colors.white,
    surfaceTintColor: Colors.transparent,
    insetPadding: const EdgeInsets.all(8),
    shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(6)),
    child: SizedBox(
      width: 388,
      height: 136,
      child: DefaultTextStyle.merge(
        style: const TextStyle(fontSize: 13, color: Colors.black, height: 1.2),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              height: 40,
              child: Row(
                children: [
                  const SizedBox(width: 38),
                  const Expanded(child: Text('关闭确认')),
                  IconButton(
                    tooltip: '关闭',
                    onPressed: () => _answer(false),
                    icon: const Icon(
                      Icons.close,
                      size: 16,
                      color: Colors.black,
                    ),
                  ),
                ],
              ),
            ),
            const Padding(
              padding: EdgeInsets.only(left: 14, top: 7),
              child: Text('确认退出截图？'),
            ),
            const Spacer(),
            Padding(
              padding: const EdgeInsets.fromLTRB(10, 0, 8, 8),
              child: Row(
                children: [
                  InkWell(
                    onTap: () => setState(() => _dontAsk = !_dontAsk),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        SizedBox(
                          width: 18,
                          height: 18,
                          child: Checkbox(
                            value: _dontAsk,
                            onChanged: (value) =>
                                setState(() => _dontAsk = value!),
                            materialTapTargetSize:
                                MaterialTapTargetSize.shrinkWrap,
                            side: const BorderSide(color: Color(0xffdedede)),
                            shape: RoundedRectangleBorder(
                              borderRadius: BorderRadius.circular(4),
                            ),
                          ),
                        ),
                        const SizedBox(width: 4),
                        const Text('不再询问'),
                      ],
                    ),
                  ),
                  const Spacer(),
                  SizedBox(
                    width: 48,
                    height: 32,
                    child: FilledButton(
                      onPressed: () => _answer(true),
                      style: FilledButton.styleFrom(
                        padding: EdgeInsets.zero,
                        backgroundColor: const Color(0xff3385ff),
                        foregroundColor: Colors.white,
                        shape: RoundedRectangleBorder(
                          borderRadius: BorderRadius.circular(4),
                        ),
                      ),
                      child: const Text('是', style: TextStyle(fontSize: 13)),
                    ),
                  ),
                  const SizedBox(width: 6),
                  SizedBox(
                    width: 48,
                    height: 32,
                    child: OutlinedButton(
                      onPressed: () => _answer(false),
                      style: OutlinedButton.styleFrom(
                        padding: EdgeInsets.zero,
                        foregroundColor: Colors.black,
                        side: const BorderSide(color: Color(0xffdedede)),
                        shape: RoundedRectangleBorder(
                          borderRadius: BorderRadius.circular(4),
                        ),
                      ),
                      child: const Text('否', style: TextStyle(fontSize: 13)),
                    ),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    ),
  );
}
