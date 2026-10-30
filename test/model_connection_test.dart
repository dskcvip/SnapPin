import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/app_config.dart';
import 'package:snappin_flutter/translation_service.dart';

void main() {
  test('connection probe matches Tauri and accepts HTTP success without a translation', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    final received = server.first.then((request) async {
      expect(request.uri.path, '/chat/completions');
      expect(
        request.headers.value(HttpHeaders.authorizationHeader),
        'Bearer test-key',
      );
      expect(jsonDecode(await utf8.decoder.bind(request).join()), {
        'model': 'example',
        'max_tokens': 1,
        'messages': [
          {'role': 'user', 'content': 'hi'},
        ],
      });
      request.response.statusCode = 204;
      await request.response.close();
    });
    await TranslationService.testConnection(
      TranslationModel(
        id: 'probe',
        name: 'Probe',
        baseUrl: ' http://127.0.0.1:${server.port}/ ',
        apiKey: ' test-key ',
        model: 'example',
      ),
    );
    await received;
  });

  test('missing API key fails before making a request', () async {
    await expectLater(
      TranslationService.testConnection(
        const TranslationModel(
          id: 'probe',
          name: 'Probe',
          baseUrl: 'invalid',
          apiKey: ' ',
          model: 'example',
        ),
      ),
      throwsA(
        isA<FormatException>().having(
          (e) => e.message,
          'message',
          '还没有填写 API Key',
        ),
      ),
    );
  });

  for (final entry in {
    401: 'API Key 无效（HTTP 401）：test failure',
    402: '余额不足（HTTP 402）：test failure',
    404: '模型或地址不存在（HTTP 404）：检查模型 ID 与 Base URL',
    429: '服务端返回 HTTP 429：test failure',
  }.entries) {
    test(
      'connection probe reports Tauri error for HTTP ${entry.key}',
      () async {
        final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
        addTearDown(() => server.close(force: true));
        final response = server.first.then((request) async {
          await request.drain<void>();
          request.response.statusCode = entry.key;
          request.response.write(
            jsonEncode({
              'error': {'message': 'test failure'},
            }),
          );
          await request.response.close();
        });
        await expectLater(
          TranslationService.testConnection(
            TranslationModel(
              id: 'probe',
              name: 'Probe',
              baseUrl: 'http://127.0.0.1:${server.port}',
              apiKey: 'test-key',
              model: 'example',
            ),
          ),
          throwsA(
            isA<FormatException>().having(
              (e) => e.message,
              'message',
              entry.value,
            ),
          ),
        );
        await response;
      },
    );
  }
}
