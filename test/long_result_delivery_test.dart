import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/long_result_delivery.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  const windows = MethodChannel('mixin.one/desktop_multi_window');
  const channels = MethodChannel('mixin.one/desktop_multi_window/channels');

  test('failed creation and delayed engine readiness preserve result for retry without duplicate windows', () async {
    final delivery = LongResultDelivery()
      ..pending = {
        'role': 'pin',
        'path': 'retained-long.png',
        'width': 837,
        'height': 45000,
      };
    var attempts = 0;
    var ready = false;
    messenger.setMockMethodCallHandler(windows, (call) async {
      if (call.method == 'createWindow') {
        attempts++;
        if (attempts == 1) throw PlatformException(code: 'injected');
        return 'long-result-window';
      }
      if (call.method == 'getAllWindows')
        return [
          {
            'windowId': 'long-result-window',
            'windowArgument': '{"role":"pin"}',
          },
        ];
      return null;
    });
    messenger.setMockMethodCallHandler(channels, (call) async {
      if (call.method == 'invokeMethod') {
        if (!ready) throw PlatformException(code: 'CHANNEL_NOT_FOUND');
        return 'retained-long.png';
      }
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(windows, null);
      messenger.setMockMethodCallHandler(channels, null);
    });
    await expectLater(delivery.open(), throwsA(isA<PlatformException>()));
    expect(delivery.pending?['height'], 45000);
    await expectLater(
      delivery.open(readyTimeout: const Duration(milliseconds: 70)),
      throwsStateError,
    );
    expect(delivery.pending?['width'], 837);
    expect(attempts, 2);
    ready = true;
    await delivery.open();
    expect(attempts, 2);
    expect(delivery.pending, isNull);
  });
}
