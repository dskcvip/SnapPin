import 'dart:io';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:image/image.dart' as img;

enum LongStepKind { appended, noMatch, still, rewound }

class LongCaptureLimitException implements Exception {
  const LongCaptureLimitException();

  @override
  String toString() => '长截图已达到内存保护上限';
}

class LongStep {
  const LongStep(
    this.kind,
    this.appended,
    this.stillCount,
    this.height,
    this.steps,
    this.score,
    this.diff, {
    this.direction = 'none',
  });

  final LongStepKind kind;
  final int appended, stillCount, height, steps;
  final double score, diff;
  final String direction;
}

class LongStitchSession {
  LongStitchSession._(
    this.width,
    this._frameHeight,
    this._canvas,
    this._previous,
    this._maxCanvasBytes,
  ) : _sourceWidth = width,
      _sourceHeight = _frameHeight;

  int width;
  final int _frameHeight;
  final int _sourceWidth;
  final int _sourceHeight;
  final int _maxCanvasBytes;
  Uint8List _canvas;
  Uint8List _previous;
  int _height = 0;
  int _steps = 1;
  int _stillCount = 0;
  int _viewportTop = 0;
  int _origin = 0;
  bool _previousLocated = true;
  final List<Uint8List> _detachedFrames = [];

  static LongStitchSession start(
    Uint8List png, {
    int maxCanvasBytes = 512 * 1024 * 1024,
  }) {
    final image = img.decodePng(png);
    if (image == null || image.height < 80) {
      throw StateError('长截图首帧无效');
    }
    final rgba = Uint8List.fromList(
      image.getBytes(order: img.ChannelOrder.rgba),
    );
    if (maxCanvasBytes < rgba.length) {
      throw ArgumentError.value(maxCanvasBytes, 'maxCanvasBytes');
    }
    return LongStitchSession._(
      image.width,
      image.height,
      rgba,
      rgba,
      maxCanvasBytes,
    ).._height = image.height;
  }

  int get height => _height;
  int get steps => _steps;
  double get previewScale => math.min(1.0, math.min(300 / width, 540 / height));

  LongStep push(Uint8List png) {
    final image = img.decodePng(png);
    if (image == null ||
        image.width != _sourceWidth ||
        image.height != _sourceHeight) {
      throw StateError('长截图采集帧尺寸变化');
    }
    final raw = image.getBytes(order: img.ChannelOrder.rgba);
    return _pushRaw(raw, sourcePng: png);
  }

  LongStep _pushRaw(
    Uint8List raw, {
    Uint8List? sourcePng,
    bool replay = false,
  }) {
    final next = raw;
    if (!replay) _steps++;
    final frameDiff = _difference(_previous, next, 0, 0, _frameHeight);
    if (frameDiff < 1.0) {
      _stillCount++;
      return LongStep(
        LongStepKind.still,
        0,
        _stillCount,
        _height,
        _steps,
        1 - frameDiff / 255,
        frameDiff,
      );
    }
    _stillCount = 0;

    // A viewport moving through rows already on the canvas must not be
    // appended again when the user scrolls upward or resumes after a gap.
    final capturedTop = replay ? null : _findCapturedTop(next);
    if (capturedTop != null) {
      _viewportTop = capturedTop;
      _previous = Uint8List.fromList(next);
      _previousLocated = true;
      final heightBeforeReplay = _height;
      final originBeforeReplay = _origin;
      if (!replay && _detachedFrames.isNotEmpty) {
        final frames = List<Uint8List>.of(_detachedFrames.reversed);
        _detachedFrames.clear();
        for (final png in frames) {
          final image = img.decodePng(png)!;
          final step = _pushRaw(
            image.getBytes(order: img.ChannelOrder.rgba),
            replay: true,
          );
          if (step.kind == LongStepKind.noMatch) break;
        }
        final added = _height - heightBeforeReplay;
        final prepended = originBeforeReplay - _origin;
        _viewportTop = capturedTop + prepended;
        _previous = Uint8List.fromList(next);
        _previousLocated = true;
        if (added > 0) {
          return LongStep(
            LongStepKind.appended,
            added,
            0,
            _height,
            _steps,
            1,
            frameDiff,
            direction: prepended == added
                ? 'recovered-prepend'
                : prepended == 0
                ? 'recovered-append'
                : 'recovered-both',
          );
        }
      }
      return LongStep(
        LongStepKind.rewound,
        0,
        0,
        _height,
        _steps,
        1,
        frameDiff,
        direction: 'existing',
      );
    }

    if (_previousLocated) {
      final upward = _frameMotion(next, _previous);
      final downward = _frameMotion(_previous, next);
      if (upward.shift != 0 &&
          (downward.shift == 0 ||
              upward.support > downward.support ||
              (upward.support == downward.support &&
                  upward.diff < downward.diff))) {
        final newTop = _viewportTop - upward.shift;
        final prepend = math.max(0, -newTop);
        if (prepend > 0) {
          _checkCapacity(_height + prepend);
          final extended = Uint8List((_height + prepend) * width * 4);
          extended.setRange(0, prepend * width * 4, next);
          extended.setRange(prepend * width * 4, extended.length, _canvas, 0);
          _canvas = extended;
          _height += prepend;
          _origin -= prepend;
        }
        _viewportTop = math.max(0, newTop);
        _previous = Uint8List.fromList(next);
        return LongStep(
          prepend > 0 ? LongStepKind.appended : LongStepKind.rewound,
          prepend,
          0,
          _height,
          _steps,
          1 - upward.diff / 255,
          frameDiff,
          direction: prepend > 0 ? 'prepend-up' : 'up-existing',
        );
      }
      if (downward.shift != 0 &&
          _viewportTop + downward.shift + _frameHeight <= _height) {
        _viewportTop += downward.shift;
        _previous = Uint8List.fromList(next);
        return LongStep(
          LongStepKind.rewound,
          0,
          0,
          _height,
          _steps,
          1 - downward.diff / 255,
          frameDiff,
          direction: 'down-existing',
        );
      }
    }

    // A scrollbar jump can skip every intermediate row. Once detached, a
    // coincidental overlap with the tail is not evidence of downward scroll.
    if (!_previousLocated) {
      _previous = Uint8List.fromList(next);
      if (!replay && sourcePng != null) {
        _detachedFrames.add(Uint8List.fromList(sourcePng));
        if (_detachedFrames.length > 256) _detachedFrames.removeAt(0);
      }
      return LongStep(
        LongStepKind.noMatch,
        0,
        0,
        _height,
        _steps,
        0,
        frameDiff,
        direction: 'awaiting-anchor',
      );
    }

    final minimumOverlap = (_frameHeight ~/ 6).clamp(24, 120);
    final maximumShift = _frameHeight - minimumOverlap;
    var bestShift = 0;
    var bestSupport = 0;
    var bestDiff = double.infinity;
    final candidates = _candidateShifts(next, maximumShift);
    void checkShift(int shift) {
      final overlap = _frameHeight - shift;
      final match = _overlapMatch(next, overlap);
      if (match.support > bestSupport ||
          (match.support == bestSupport && match.diff < bestDiff)) {
        bestSupport = match.support;
        bestDiff = match.diff;
        bestShift = shift;
      }
    }

    for (final shift in candidates) {
      checkShift(shift);
    }
    if (bestSupport < 24) {
      for (var shift = 1; shift <= maximumShift; shift++) {
        if (!candidates.contains(shift)) checkShift(shift);
      }
    }
    if (bestSupport < 24 || bestShift == 0) {
      _previous = Uint8List.fromList(next);
      _previousLocated = false;
      if (!replay && sourcePng != null) {
        _detachedFrames.add(Uint8List.fromList(sourcePng));
      }
      return LongStep(
        LongStepKind.noMatch,
        0,
        0,
        _height,
        _steps,
        0,
        frameDiff,
        direction: 'unmatched',
      );
    }
    _ensureCapacity(_height + bestShift);
    final firstNewRow = _frameHeight - bestShift;
    _canvas.setRange(
      _height * width * 4,
      (_height + bestShift) * width * 4,
      next,
      firstNewRow * width * 4,
    );
    _height += bestShift;
    _viewportTop = _height - _frameHeight;
    _previous = Uint8List.fromList(next);
    _previousLocated = true;
    return LongStep(
      LongStepKind.appended,
      bestShift,
      0,
      _height,
      _steps,
      1 - bestDiff / 255,
      frameDiff,
      direction: 'append-down',
    );
  }

  ({int shift, int support, double diff, int start}) _frameMotion(
    Uint8List first,
    Uint8List second,
  ) {
    var bestShift = 0;
    var bestSupport = 0;
    var bestDiff = double.infinity;
    var bestStart = 0;
    final maximumShift = _frameHeight - (_frameHeight ~/ 6).clamp(24, 120);
    for (final shift in _candidateShiftsBetween(
      first,
      0,
      second,
      0,
      maximumShift,
    )) {
      final match = _matchRows(first, shift, second, 0, _frameHeight - shift);
      if (match.support > bestSupport ||
          (match.support == bestSupport && match.diff < bestDiff)) {
        bestShift = shift;
        bestSupport = match.support;
        bestDiff = match.diff;
        bestStart = match.start;
      }
    }
    return bestSupport >= 24
        ? (
            shift: bestShift,
            support: bestSupport,
            diff: bestDiff,
            start: bestStart,
          )
        : (shift: 0, support: 0, diff: double.infinity, start: 0);
  }

  List<int> _candidateShifts(Uint8List frame, int maximumShift) {
    return _candidateShiftsBetween(
      _canvas,
      _height - _frameHeight,
      frame,
      0,
      maximumShift,
    );
  }

  List<int> _candidateShiftsBetween(
    Uint8List first,
    int firstTop,
    Uint8List second,
    int secondTop,
    int maximumShift,
  ) {
    final rowsByHash = <int, List<int>>{};
    for (var row = 0; row < _frameHeight; row++) {
      final hash = _rowHash(first, firstTop + row);
      if (hash != null) rowsByHash.putIfAbsent(hash, () => []).add(row);
    }
    final votes = <int, int>{};
    for (var row = 0; row < _frameHeight; row += 8) {
      final hash = _rowHash(second, secondTop + row);
      if (hash == null) continue;
      for (final tailRow in rowsByHash[hash] ?? const <int>[]) {
        final shift = tailRow - row;
        if (shift > 0 && shift <= maximumShift) {
          votes[shift] = (votes[shift] ?? 0) + 1;
        }
      }
    }
    final ranked = votes.keys.toList()
      ..sort((a, b) => votes[b]!.compareTo(votes[a]!));
    return ranked.take(24).toList();
  }

  int? _rowHash(Uint8List pixels, int row) {
    final stride = math.max(1, width ~/ 16);
    final start = row * width * 4;
    final base = pixels[start];
    var texture = 0;
    var hash = 2166136261;
    var count = 0;
    for (var x = 0; x < width; x += stride) {
      final index = start + x * 4;
      texture += (pixels[index] - base).abs();
      for (var channel = 0; channel < 3; channel++) {
        hash = ((hash ^ pixels[index + channel]) * 16777619) & 0xffffffff;
      }
      count++;
    }
    return texture / count > 6 ? hash : null;
  }

  void _checkCapacity(int rows) {
    if (rows * width * 4 > _maxCanvasBytes) {
      throw const LongCaptureLimitException();
    }
  }

  void _ensureCapacity(int rows) {
    _checkCapacity(rows);
    final required = rows * width * 4;
    if (required <= _canvas.length) return;
    final capacity = math.min(
      _maxCanvasBytes,
      math.max(required, _canvas.length + (_canvas.length ~/ 4)),
    );
    final extended = Uint8List(capacity);
    extended.setRange(0, _height * width * 4, _canvas);
    _canvas = extended;
  }

  ({int support, double diff, int start}) _overlapMatch(
    Uint8List frame,
    int rows,
  ) => _matchRows(_canvas, _height - rows, frame, 0, rows);

  ({int support, double diff, int start}) _matchRows(
    Uint8List first,
    int firstTop,
    Uint8List second,
    int secondTop,
    int rows,
  ) {
    var run = 0;
    var evidence = 0;
    var runDiff = 0.0;
    var bestRun = 0;
    var bestDiff = double.infinity;
    var bestStart = 0;
    final colStep = math.max(1, width ~/ 48);
    for (var y = 0; y < rows; y += 4) {
      final canvasRow = (firstTop + y) * width * 4;
      final frameRow = (secondTop + y) * width * 4;
      var diff = 0;
      var texture = 0;
      var count = 0;
      final base = second[frameRow].toInt();
      for (var x = 0; x < width; x += colStep) {
        final a = canvasRow + x * 4;
        final b = frameRow + x * 4;
        diff +=
            (first[a] - second[b]).abs() +
            (first[a + 1] - second[b + 1]).abs() +
            (first[a + 2] - second[b + 2]).abs();
        texture += (second[b] - base).abs();
        count++;
      }
      final mean = diff / (count * 3);
      if (mean < 7) {
        run += 4;
        if (texture / count > 6) evidence++;
        runDiff += mean;
        if (evidence >= 4 && run >= 24 && run > bestRun) {
          bestRun = run;
          bestDiff = runDiff / (run / 4);
          bestStart = y + 4 - run;
        }
      } else {
        run = 0;
        evidence = 0;
        runDiff = 0;
      }
    }
    return (support: bestRun, diff: bestDiff, start: bestStart);
  }

  int? _findCapturedTop(Uint8List frame) {
    if (_height <= _frameHeight) return null;
    final lastTop = _height - _frameHeight;
    // Check the anchored viewport first, then the rest of the captured image.
    if (_viewportTop <= lastTop &&
        _difference(_canvas, frame, _viewportTop, 0, _frameHeight) < 2) {
      return _viewportTop;
    }
    // Exact row hashes locate viewports at arbitrary pixel offsets. The old
    // four-pixel scan misses positions between its probes after a scrollbar
    // drag, especially when the pointer or page content changes slightly.
    if (!_previousLocated) {
      final rowsByHash = <int, List<int>>{};
      for (var row = 0; row < _height; row++) {
        final hash = _rowHash(_canvas, row);
        if (hash != null) rowsByHash.putIfAbsent(hash, () => []).add(row);
      }
      final votes = <int, int>{};
      for (var row = 0; row < _frameHeight; row += 8) {
        final hash = _rowHash(frame, row);
        if (hash == null) continue;
        for (final canvasRow in rowsByHash[hash] ?? const <int>[]) {
          final top = canvasRow - row;
          if (top >= 0 && top <= lastTop) {
            votes[top] = (votes[top] ?? 0) + 1;
          }
        }
      }
      final ranked = votes.keys.toList()
        ..sort((a, b) => votes[b]!.compareTo(votes[a]!));
      for (final top in ranked.take(16)) {
        if (votes[top]! >= 4 &&
            _difference(_canvas, frame, top, 0, _frameHeight) < 2) {
          return top;
        }
      }
    }
    for (var top = 0; top <= lastTop; top += 4) {
      if (top == _viewportTop) continue;
      if (_quickDifference(_canvas, frame, top) < 3 &&
          _difference(_canvas, frame, top, 0, _frameHeight) < 2) {
        return top;
      }
    }
    if (lastTop % 4 != 0 &&
        _difference(_canvas, frame, lastTop, 0, _frameHeight) < 2) {
      return lastTop;
    }
    return null;
  }

  double _quickDifference(Uint8List canvas, Uint8List frame, int top) {
    var total = 0;
    var samples = 0;
    for (final y in [
      0,
      _frameHeight ~/ 3,
      _frameHeight * 2 ~/ 3,
      _frameHeight - 1,
    ]) {
      for (var x = 0; x < width; x += math.max(1, width ~/ 8)) {
        final a = ((top + y) * width + x) * 4;
        final b = (y * width + x) * 4;
        total +=
            (canvas[a] - frame[b]).abs() +
            (canvas[a + 1] - frame[b + 1]).abs() +
            (canvas[a + 2] - frame[b + 2]).abs();
        samples += 3;
      }
    }
    return total / samples;
  }

  double _difference(Uint8List a, Uint8List b, int aRow, int bRow, int rows) {
    final rowStride = math.max(1, rows ~/ 32);
    final colStride = math.max(1, width ~/ 64);
    var total = 0;
    var samples = 0;
    for (var y = 0; y < rows; y += rowStride) {
      final left = (aRow + y) * width * 4;
      final right = (bRow + y) * width * 4;
      for (var x = 0; x < width; x += colStride) {
        final ai = left + x * 4;
        final bi = right + x * 4;
        total +=
            (a[ai] - b[bi]).abs() +
            (a[ai + 1] - b[bi + 1]).abs() +
            (a[ai + 2] - b[bi + 2]).abs();
        samples += 3;
      }
    }
    return total / samples;
  }

  Uint8List previewPng() {
    final scale = previewScale;
    final previewWidth = math.max(1, (width * scale).round());
    final previewHeight = math.max(1, (height * scale).round());
    final output = Uint8List(previewWidth * previewHeight * 4);
    for (var y = 0; y < previewHeight; y++) {
      final sourceY = y * height ~/ previewHeight;
      for (var x = 0; x < previewWidth; x++) {
        final source = (sourceY * width + x * width ~/ previewWidth) * 4;
        final target = (y * previewWidth + x) * 4;
        output.setRange(target, target + 4, _canvas, source);
      }
    }
    return Uint8List.fromList(
      img.encodePng(
        img.Image.fromBytes(
          width: previewWidth,
          height: previewHeight,
          bytes: output.buffer,
          order: img.ChannelOrder.rgba,
        ),
      ),
    );
  }

  Future<File> savePng(File target) async {
    await target.parent.create(recursive: true);
    final png = img.encodePng(
      img.Image.fromBytes(
        width: width,
        height: height,
        bytes: Uint8List.sublistView(_canvas, 0, _height * width * 4).buffer,
        order: img.ChannelOrder.rgba,
      ),
    );
    await target.writeAsBytes(png);
    return target;
  }

  Uint8List firstTilePng() {
    final rows = math.min(1024, height);
    return Uint8List.fromList(
      img.encodePng(
        img.Image.fromBytes(
          width: width,
          height: rows,
          bytes: Uint8List.sublistView(_canvas, 0, rows * width * 4).buffer,
          order: img.ChannelOrder.rgba,
        ),
        level: 1,
      ),
    );
  }

  void dispose() {
    _canvas = Uint8List(0);
    _previous = Uint8List(0);
    _detachedFrames.clear();
  }
}
