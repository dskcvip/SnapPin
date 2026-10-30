import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/capture_bridge.dart';

void main() {
  const display = CapturedDisplay(
    path: 'capture.png',
    x: -1680,
    y: 0,
    width: 1680,
    height: 1050,
    pixelWidth: 3360,
    pixelHeight: 2100,
    pixelX: -3360,
    pixelY: 0,
    scale: 2,
  );
  const selection = Rect.fromLTWH(100, 50, 800, 600);

  test('Windows long capture uses physical pixels', () {
    final region = CaptureBridge.longCaptureRegion(
      display,
      selection,
      macOS: false,
    );
    expect(region['x'], -3156);
    expect(region['y'], 104);
    expect(region['width'], 1592);
    expect(region['height'], 1192);
    expect(region['logicalX'], -1580);
  });

  test('macOS long capture uses logical points on Retina displays', () {
    final region = CaptureBridge.longCaptureRegion(
      display,
      selection,
      macOS: true,
    );
    expect(region['x'], -1576);
    expect(region['y'], 54);
    expect(region['width'], 792);
    expect(region['height'], 592);
    expect(region['logicalX'], -1580);
  });
}
