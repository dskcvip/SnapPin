import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:snappin_flutter/app_config.dart';
import 'package:snappin_flutter/translation_service.dart';

void main() {
  late HttpServer server;
  late TranslationModel model;

  setUp(() async {
    server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    model = TranslationModel(
      id: 'local',
      name: 'Local',
      baseUrl: 'http://127.0.0.1:${server.port}',
      apiKey: 'test-key',
      model: 'example',
    );
  });
  tearDown(() => server.close(force: true));

  for (final sendHeaders in [false, true]) {
    test(
      'translation times out with stalled ${sendHeaders ? 'body' : 'headers'}',
      () async {
        final serving = server.first.then((request) async {
          await request.drain<void>();
          if (sendHeaders) {
            request.response.write('{');
            await request.response.flush();
          }
          // Keep the response open, as a stalled provider would.
        });
        await expectLater(
          TranslationService.translate(
            'hello',
            model,
            '中文',
            requestTimeout: const Duration(seconds: 1),
          ),
          throwsA(
            isA<FormatException>().having(
              (e) => e.message,
              'message',
              '翻译请求超时，请重试',
            ),
          ),
        );
        await serving;
      },
    );
  }

  test('translation preserves returned line breaks', () async {
    final serving = server.first.then((request) async {
      final body = jsonDecode(await utf8.decoder.bind(request).join()) as Map;
      expect((body['messages'] as List).last['content'], 'hello\nworld');
      request.response.write(
        jsonEncode({
          'choices': [
            {
              'message': {'content': '你好\n世界'},
            },
          ],
        }),
      );
      await request.response.close();
    });
    expect(
      await TranslationService.translate('hello\nworld', model, '中文'),
      '你好\n世界',
    );
    await serving;
  });

  for (final body in [
    <String, Object>{},
    {'choices': <Object>[]},
    {
      'choices': [null],
    },
    {
      'choices': [
        {
          'message': {'content': ' '},
        },
      ],
    },
    {
      'choices': [
        {
          'message': {'content': 42},
        },
      ],
    },
  ]) {
    test(
      'invalid translation reports a readable error: ${jsonEncode(body)}',
      () async {
        final serving = server.first.then((request) async {
          await request.drain<void>();
          request.response.write(jsonEncode(body));
          await request.response.close();
        });
        await expectLater(
          TranslationService.translate('hello', model, '中文'),
          throwsA(
            isA<FormatException>().having(
              (e) => e.message,
              'message',
              '翻译响应为空或格式不正确',
            ),
          ),
        );
        await serving;
      },
    );
  }
}
