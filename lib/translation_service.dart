import 'dart:convert';
import 'dart:io';

import 'app_config.dart';

class TranslationService {
  static Future<String> translate(
    String source,
    TranslationModel model,
    String targetLanguage,
  ) async {
    if (model.apiKey.isEmpty || model.model.isEmpty || model.baseUrl.isEmpty) {
      throw StateError('翻译模型配置不完整');
    }
    final base = model.baseUrl.replaceFirst(RegExp(r'/+$'), '');
    final uri = Uri.parse('$base/chat/completions');
    final client = HttpClient()..connectionTimeout = const Duration(seconds: 15);
    try {
      final request = await client.postUrl(uri);
      request.headers.contentType = ContentType.json;
      request.headers.set(HttpHeaders.authorizationHeader, 'Bearer ${model.apiKey}');
      request.write(jsonEncode({
        'model': model.model,
        'temperature': 0,
        'messages': [
          {
            'role': 'system',
            'content': 'Translate the user text into $targetLanguage. '
                'Return only the translation, preserving line breaks.',
          },
          {'role': 'user', 'content': source},
        ],
      }));
      final response = await request.close().timeout(const Duration(seconds: 45));
      final body = await utf8.decoder.bind(response).join();
      if (response.statusCode < 200 || response.statusCode >= 300) {
        throw HttpException('翻译请求失败：${response.statusCode}', uri: uri);
      }
      final data = jsonDecode(body) as Map<String, dynamic>;
      final choices = data['choices'] as List?;
      if (choices == null || choices.isEmpty) throw FormatException('翻译响应为空');
      final message = (choices.first as Map)['message'] as Map;
      return message['content'] as String? ?? '';
    } finally {
      client.close(force: true);
    }
  }
}
