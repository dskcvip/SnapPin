import 'dart:convert';
import 'dart:async';
import 'dart:io';

import 'app_config.dart';
import 'app_log.dart';

class TranslationService {
  /// Same minimal probe as Tauri: a successful HTTP status is sufficient.
  static Future<void> testConnection(TranslationModel model) async {
    if (model.apiKey.trim().isEmpty) {
      throw const FormatException('还没有填写 API Key');
    }
    final base = model.baseUrl.trim().replaceFirst(RegExp(r'/+$'), '');
    final client = HttpClient()
      ..connectionTimeout = const Duration(seconds: 20);
    try {
      await (() async {
        final request = await client.postUrl(
          Uri.parse('$base/chat/completions'),
        );
        request.headers.contentType = ContentType.json;
        request.headers.set(
          HttpHeaders.authorizationHeader,
          'Bearer ${model.apiKey.trim()}',
        );
        request.write(
          jsonEncode({
            'model': model.model,
            'max_tokens': 1,
            'messages': [
              {'role': 'user', 'content': 'hi'},
            ],
          }),
        );
        final response = await request.close();
        if (response.statusCode >= 200 && response.statusCode < 300) return;
        String message = '（服务端未给出原因）';
        try {
          final parsed = jsonDecode(await utf8.decoder.bind(response).join());
          final detail = parsed is Map ? parsed['error'] : null;
          if (detail is Map && detail['message'] is String)
            message = detail['message'] as String;
        } catch (_) {}
        final status = response.statusCode;
        throw FormatException(switch (status) {
          401 => 'API Key 无效（HTTP 401）：$message',
          402 => '余额不足（HTTP 402）：$message',
          404 => '模型或地址不存在（HTTP 404）：检查模型 ID 与 Base URL',
          _ => '服务端返回 HTTP $status：$message',
        });
      })().timeout(const Duration(seconds: 20));
    } on TimeoutException {
      throw FormatException('连接失败：请求超时（${model.baseUrl}）');
    } on SocketException {
      throw FormatException('连接失败：无法连接服务（${model.baseUrl}）');
    } on HttpException {
      throw FormatException('连接失败：HTTP 连接异常（${model.baseUrl}）');
    } finally {
      client.close(force: true);
    }
  }

  static Future<String> translate(
    String source,
    TranslationModel model,
    String targetLanguage, {
    Duration requestTimeout = const Duration(seconds: 45),
  }) async {
    if (model.apiKey.isEmpty || model.model.isEmpty || model.baseUrl.isEmpty) {
      throw StateError('翻译模型配置不完整');
    }
    final base = model.baseUrl.replaceFirst(RegExp(r'/+$'), '');
    final uri = Uri.parse('$base/chat/completions');
    final client = HttpClient()
      ..connectionTimeout = const Duration(seconds: 15);
    final elapsed = Stopwatch()..start();
    AppLog.write('translation-start', 'sourceChars=${source.length}');
    try {
      // Bound the entire exchange, including response-body streaming. A server
      // can send headers promptly and then never finish its JSON response.
      return await (() async {
        final request = await client.postUrl(uri);
        request.headers.contentType = ContentType.json;
        request.headers.set(
          HttpHeaders.authorizationHeader,
          'Bearer ${model.apiKey}',
        );
        request.write(
          jsonEncode({
            'model': model.model,
            'temperature': 0,
            'messages': [
              {
                'role': 'system',
                'content':
                    'Translate the user text into $targetLanguage. '
                    'Return only the translation, preserving line breaks.',
              },
              {'role': 'user', 'content': source},
            ],
          }),
        );
        final response = await request.close();
        final body = await utf8.decoder.bind(response).join();
        if (response.statusCode < 200 || response.statusCode >= 300) {
          throw HttpException('翻译请求失败：${response.statusCode}', uri: uri);
        }
        final data = jsonDecode(body);
        final choices = data is Map ? data['choices'] : null;
        final first = choices is List && choices.isNotEmpty
            ? choices.first
            : null;
        final message = first is Map ? first['message'] : null;
        final content = message is Map ? message['content'] : null;
        if (content is! String || content.trim().isEmpty) {
          throw const FormatException('翻译响应为空或格式不正确');
        }
        AppLog.write(
          'translation-ready',
          'resultChars=${content.length} elapsed=${elapsed.elapsedMilliseconds}ms',
        );
        return content;
      })().timeout(requestTimeout);
    } on TimeoutException {
      AppLog.write(
        'translation-timeout',
        'elapsed=${elapsed.elapsedMilliseconds}ms',
      );
      throw const FormatException('翻译请求超时，请重试');
    } catch (error) {
      AppLog.write(
        'translation-failed',
        'type=${error.runtimeType} elapsed=${elapsed.elapsedMilliseconds}ms',
      );
      rethrow;
    } finally {
      client.close(force: true);
    }
  }
}
