import 'dart:async';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'long_stitch.dart';

/// Keeps matching, the full resolution canvas and PNG encoding off the UI isolate.
class LongStitchWorker {
  LongStitchWorker._();

  final _responses = ReceivePort();
  final _errors = ReceivePort();
  final _exits = ReceivePort();
  final _ready = Completer<SendPort>();
  final _pending = <int, Completer<Object?>>{};
  Isolate? _isolate;
  SendPort? _commands;
  int _sequence = 0;
  bool _disposed = false;
  int width = 0;
  int height = 0;
  int steps = 0;
  double previewScale = 1;

  static Future<LongStitchWorker> start(
    Uint8List png, {
    int maxCanvasBytes = 512 * 1024 * 1024,
  }) async {
    final worker = LongStitchWorker._();
    // Also observe startup failures if spawning throws before awaiting readiness.
    worker._ready.future.ignore();
    worker._responses.listen(worker._receive);
    worker._errors.listen((error) {
      worker._fail(StateError('长截图处理线程异常: $error'));
    });
    worker._exits.listen((_) {
      worker._fail(StateError('长截图处理线程已结束'));
    });
    try {
      worker._isolate = await Isolate.spawn(
        _run,
        worker._responses.sendPort,
        onError: worker._errors.sendPort,
        onExit: worker._exits.sendPort,
        debugName: 'snappin-long-stitch',
      );
      worker._commands = await worker._ready.future;
      await worker._request('start', (png, maxCanvasBytes));
      return worker;
    } catch (_) {
      worker.dispose();
      rethrow;
    }
  }

  void _receive(dynamic message) {
    if (message is SendPort) {
      if (!_ready.isCompleted) _ready.complete(message);
      return;
    }
    final (id, success, payload, state) =
        message as (int, bool, Object?, (int, int, int, double)?);
    if (state != null) {
      width = state.$1;
      height = state.$2;
      steps = state.$3;
      previewScale = state.$4;
    }
    final completion = _pending.remove(id);
    if (completion == null) return;
    if (success) {
      completion.complete(payload);
    } else {
      final (limit, error, stack) = payload as (bool, String, String);
      completion.completeError(
        limit ? const LongCaptureLimitException() : StateError(error),
        StackTrace.fromString(stack),
      );
    }
  }

  Future<Object?> _request(String command, [Object? payload]) {
    if (_disposed) return Future.error(StateError('长截图处理线程已关闭'));
    final id = ++_sequence;
    final completion = Completer<Object?>();
    _pending[id] = completion;
    _commands!.send((id, command, payload));
    return completion.future;
  }

  Future<LongStep> push(Uint8List png) async =>
      await _request('push', png) as LongStep;

  Future<Uint8List> previewPng() async =>
      await _request('preview') as Uint8List;

  Future<Uint8List> firstTilePng() async =>
      await _request('firstTile') as Uint8List;

  Future<File> savePng(File target) async {
    await _request('save', target.path);
    return target;
  }

  void _fail(Object error) {
    if (!_ready.isCompleted) _ready.completeError(error);
    for (final completion in _pending.values) {
      completion.completeError(error);
    }
    _pending.clear();
    _disposed = true;
  }

  void dispose() {
    _fail(StateError('长截图处理线程已关闭'));
    _isolate?.kill(priority: Isolate.immediate);
    _responses.close();
    _errors.close();
    _exits.close();
  }

  static Future<void> _run(SendPort replies) async {
    final commands = ReceivePort();
    replies.send(commands.sendPort);
    LongStitchSession? session;
    await for (final message in commands) {
      final (id, command, payload) = message as (int, String, Object?);
      try {
        final Object? result;
        switch (command) {
          case 'start':
            final (png, limit) = payload as (Uint8List, int);
            session = LongStitchSession.start(png, maxCanvasBytes: limit);
            result = null;
          case 'push':
            result = session!.push(payload as Uint8List);
          case 'preview':
            result = session!.previewPng();
          case 'firstTile':
            result = session!.firstTilePng();
          case 'save':
            await session!.savePng(File(payload as String));
            result = null;
          default:
            throw StateError('未知长截图操作: $command');
        }
        replies.send((
          id,
          true,
          result,
          (session.width, session.height, session.steps, session.previewScale),
        ));
      } catch (error, stack) {
        replies.send((
          id,
          false,
          (error is LongCaptureLimitException, error.toString(), '$stack'),
          null,
        ));
      }
    }
  }
}
