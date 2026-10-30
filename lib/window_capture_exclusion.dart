import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:window_manager/window_manager.dart';

class WindowCaptureExclusion {
  static Future<bool> enable(String title) async {
    if (!Platform.isWindows) return false;
    await windowManager.setTitle(title);
    final user32 = ffi.DynamicLibrary.open('user32.dll');
    final findWindow = user32
        .lookupFunction<
          ffi.IntPtr Function(ffi.Pointer<ffi.Uint16>, ffi.Pointer<ffi.Uint16>),
          int Function(ffi.Pointer<ffi.Uint16>, ffi.Pointer<ffi.Uint16>)
        >('FindWindowW');
    final setAffinity = user32
        .lookupFunction<
          ffi.Int32 Function(ffi.IntPtr, ffi.Uint32),
          int Function(int, int)
        >('SetWindowDisplayAffinity');
    final units = title.codeUnits;
    final nativeTitle = calloc<ffi.Uint16>(units.length + 1);
    try {
      nativeTitle.asTypedList(units.length + 1).setAll(0, [...units, 0]);
      for (var attempt = 0; attempt < 20; attempt++) {
        final handle = findWindow(ffi.nullptr, nativeTitle);
        if (handle != 0) return setAffinity(handle, 0x11) != 0;
        await Future<void>.delayed(const Duration(milliseconds: 50));
      }
      return false;
    } finally {
      calloc.free(nativeTitle);
    }
  }
}
