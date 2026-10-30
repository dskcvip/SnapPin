import 'dart:convert';
import 'dart:io';
import 'dart:ui';

class TranslationModel {
  const TranslationModel({
    required this.id,
    required this.name,
    required this.baseUrl,
    required this.apiKey,
    required this.model,
    this.note = '',
    this.keyUrl = '',
  });

  final String id, name, baseUrl, apiKey, model, note, keyUrl;

  factory TranslationModel.fromJson(Map<String, dynamic> map) =>
      TranslationModel(
        id: map['id'] as String? ?? '',
        name: map['name'] as String? ?? '',
        baseUrl: map['baseUrl'] as String? ?? '',
        apiKey: map['apiKey'] as String? ?? '',
        model: map['model'] as String? ?? '',
        note: map['note'] as String? ?? '',
        keyUrl: map['keyUrl'] as String? ?? '',
      );

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'baseUrl': baseUrl,
    'apiKey': apiKey,
    'model': model,
    'note': note,
    'keyUrl': keyUrl,
  };
}

class AppConfig {
  const AppConfig({
    this.themeColor = '#12A5D8',
    this.ballSize = 56,
    this.ballOpacity = 1,
    this.ballCenter,
    this.shortcutCapture = 'Ctrl+1',
    this.shortcutPinClipboard = 'Ctrl+2',
    this.saveDir,
    this.translationModels = const [],
    this.targetLang = '中文',
  });

  final String themeColor;
  final int ballSize;
  final double ballOpacity;
  final Offset? ballCenter;
  final String shortcutCapture;
  final String shortcutPinClipboard;
  final String? saveDir;
  final List<TranslationModel> translationModels;
  final String targetLang;

  AppConfig copyWith({
    String? themeColor,
    int? ballSize,
    double? ballOpacity,
    Offset? ballCenter,
    String? shortcutCapture,
    String? shortcutPinClipboard,
    String? saveDir,
    List<TranslationModel>? translationModels,
    String? targetLang,
  }) => AppConfig(
    themeColor: themeColor ?? this.themeColor,
    ballSize: ballSize ?? this.ballSize,
    ballOpacity: ballOpacity ?? this.ballOpacity,
    ballCenter: ballCenter ?? this.ballCenter,
    shortcutCapture: shortcutCapture ?? this.shortcutCapture,
    shortcutPinClipboard: shortcutPinClipboard ?? this.shortcutPinClipboard,
    saveDir: saveDir ?? this.saveDir,
    translationModels: translationModels ?? this.translationModels,
    targetLang: targetLang ?? this.targetLang,
  );

  factory AppConfig.fromJson(Map<String, dynamic> map) {
    final position = map['ballPosition'];
    final translation = map['translation'] is Map
        ? Map<String, dynamic>.from(map['translation'] as Map)
        : <String, dynamic>{};
    return AppConfig(
      themeColor: map['themeColor'] as String? ?? '#12A5D8',
      ballSize: ((map['ballSize'] as num?)?.toInt() ?? 56).clamp(40, 80),
      ballOpacity: ((map['ballOpacity'] as num?)?.toDouble() ?? 1).clamp(
        .15,
        1,
      ),
      ballCenter:
          position is Map && position['x'] is num && position['y'] is num
          ? Offset(
              (position['x'] as num).toDouble(),
              (position['y'] as num).toDouble(),
            )
          : null,
      shortcutCapture: map['shortcutCapture'] as String? ??
          (Platform.isMacOS ? 'Super+1' : 'Ctrl+1'),
      shortcutPinClipboard: map['shortcutPinClipboard'] as String? ??
          (Platform.isMacOS ? 'Super+2' : 'Ctrl+2'),
      saveDir: map['saveDir'] as String?,
      translationModels: (translation['models'] as List? ?? const [])
          .whereType<Map>()
          .map((model) => TranslationModel.fromJson(
              Map<String, dynamic>.from(model)))
          .toList(),
      targetLang: translation['targetLang'] as String? ?? '中文',
    );
  }

  Map<String, dynamic> toJson() => {
    'themeColor': themeColor,
    'ballSize': ballSize,
    'ballOpacity': ballOpacity,
    if (ballCenter != null)
      'ballPosition': {'x': ballCenter!.dx, 'y': ballCenter!.dy},
    'shortcutCapture': shortcutCapture,
    'shortcutPinClipboard': shortcutPinClipboard,
    'saveDir': saveDir,
    'translation': {
      'models': translationModels.map((model) => model.toJson()).toList(),
      'targetLang': targetLang,
    },
  };

  Color get color {
    final hex = themeColor.replaceFirst('#', '');
    final value = int.tryParse(hex, radix: 16) ?? 0x12A5D8;
    return Color(0xff000000 | value);
  }
}

class ConfigStore {
  static File get file {
    final root = Platform.isWindows
        ? Platform.environment['APPDATA']
        : '${Platform.environment['HOME'] ?? Directory.systemTemp.path}/Library/Application Support';
    if (root == null)
      throw StateError('Application data directory unavailable');
    return File(
      '$root${Platform.pathSeparator}com.snappin.flutter'
      '${Platform.pathSeparator}config.json',
    );
  }

  static Future<AppConfig> load() async {
    try {
      final map = jsonDecode(await file.readAsString()) as Map<String, dynamic>;
      return AppConfig.fromJson(map);
    } catch (_) {
      return AppConfig(
        shortcutCapture: Platform.isMacOS ? 'Super+1' : 'Ctrl+1',
        shortcutPinClipboard: Platform.isMacOS ? 'Super+2' : 'Ctrl+2',
      );
    }
  }

  static Future<void> save(AppConfig config) async {
    final target = file;
    await target.parent.create(recursive: true);
    final temp = File('${target.path}.tmp');
    await temp.writeAsString(jsonEncode(config.toJson()));
    if (await target.exists()) await target.delete();
    await temp.rename(target.path);
  }
}
