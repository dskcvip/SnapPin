import 'dart:convert';
import 'dart:io';
import 'dart:ui';

import 'atomic_file.dart';
import 'app_log.dart';

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
      shortcutCapture:
          map['shortcutCapture'] as String? ??
          (Platform.isMacOS ? 'Super+1' : 'Ctrl+1'),
      shortcutPinClipboard:
          map['shortcutPinClipboard'] as String? ??
          (Platform.isMacOS ? 'Super+2' : 'Ctrl+2'),
      saveDir: map['saveDir'] as String?,
      translationModels: (translation['models'] as List? ?? const [])
          .whereType<Map>()
          .map(
            (model) =>
                TranslationModel.fromJson(Map<String, dynamic>.from(model)),
          )
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
  static final _repository = ConfigRepository(file);
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

  static Future<AppConfig> load() => _repository.load();

  static Future<AppConfig> update(AppConfig Function(AppConfig) change) =>
      _repository.update(change);

  static Future<void> save(AppConfig config) async {
    await update((_) => config);
  }
}

/// All production writes are dispatched through the main (ball) engine.
/// This queue serializes read/merge/write, including drag-position updates.
class ConfigRepository {
  ConfigRepository(this.file);
  final File file;
  Future<void> _writes = Future<void>.value();

  File get _backup => File('${file.path}.bak');

  Future<AppConfig> load() async {
    for (final candidate in [file, _backup]) {
      try {
        final map =
            jsonDecode(await candidate.readAsString()) as Map<String, dynamic>;
        final config = AppConfig.fromJson(map);
        if (candidate.path != file.path) {
          AppLog.write('config-backup-recovered');
        }
        return config;
      } on FileSystemException catch (error) {
        if (await candidate.exists())
          AppLog.write('config-read-failed', error.osError?.errorCode);
      } on FormatException {
        AppLog.write('config-invalid-json', candidate.path);
      } on TypeError {
        AppLog.write('config-invalid-fields', candidate.path);
      }
    }
    return AppConfig(
      shortcutCapture: Platform.isMacOS ? 'Super+1' : 'Ctrl+1',
      shortcutPinClipboard: Platform.isMacOS ? 'Super+2' : 'Ctrl+2',
    );
  }

  Future<AppConfig> update(AppConfig Function(AppConfig) change) {
    final operation = _writes.then((_) async {
      final current = await load();
      final next = change(current);
      // Back up the parsed/recovered version, never overwrite a good backup
      // with malformed bytes from an interrupted older release.
      await atomicWrite(_backup, utf8.encode(jsonEncode(current.toJson())));
      await atomicWrite(file, utf8.encode(jsonEncode(next.toJson())));
      AppLog.write('config-saved');
      return next;
    });
    _writes = operation.then<void>(
      (_) {},
      onError: (Object _, StackTrace __) {},
    );
    return operation;
  }
}
