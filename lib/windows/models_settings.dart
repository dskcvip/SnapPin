import 'dart:io';

import 'package:flutter/material.dart';

import '../app_config.dart';
import '../app_log.dart';
import '../translation_service.dart';
import 'settings_icons.dart';

class ModelsSettings extends StatefulWidget {
  const ModelsSettings({
    super.key,
    required this.config,
    required this.onChanged,
    this.savedConfig,
  });

  final AppConfig config;
  final AppConfig? savedConfig;
  final ValueChanged<AppConfig> onChanged;

  @override
  State<ModelsSettings> createState() => _ModelsSettingsState();
}

typedef _Preset = (
  String name,
  String note,
  String url,
  String model,
  String keyUrl,
);

const List<_Preset> _presets = [
  (
    'DeepSeek',
    '深度求索',
    'https://api.deepseek.com',
    'deepseek-flash',
    'https://platform.deepseek.com/api_keys',
  ),
  (
    'Kimi',
    'Moonshot 月之暗面',
    'https://api.moonshot.cn',
    'kimi-k2-0905-preview',
    'https://platform.moonshot.cn/console/api-keys',
  ),
  (
    '智谱 GLM',
    'BigModel 开放平台',
    'https://open.bigmodel.cn/api/paas/v4',
    'glm-4-flash',
    'https://open.bigmodel.cn/usercenter/apikeys',
  ),
  (
    '通义千问',
    '阿里云百炼',
    'https://dashscope.aliyuncs.com/compatible-mode/v1',
    'qwen-flash',
    'https://bailian.console.aliyun.com/#/api-key',
  ),
  ('自定义', '任意 OpenAI 兼容端点', '', '', ''),
];

class _ModelsSettingsState extends State<ModelsSettings> {
  String? _hovered;
  String? _focused;
  String? _testing;
  String? _result;
  bool _testOk = false;

  Future<void> _edit([TranslationModel? current]) async {
    AppLog.write('model-editor-open', current == null ? 'new' : 'edit');
    setState(() => _result = null);
    final name = TextEditingController(text: current?.name ?? '');
    final note = TextEditingController(text: current?.note ?? '');
    final base = TextEditingController(
      text: current?.baseUrl ?? 'https://api.deepseek.com',
    );
    final key = TextEditingController(text: current?.apiKey ?? '');
    final model = TextEditingController(
      text: current?.model ?? 'deepseek-flash',
    );
    final keyUrl = TextEditingController(
      text: current?.keyUrl ?? 'https://platform.deepseek.com/api_keys',
    );
    var valid =
        current != null &&
        current.name.trim().isNotEmpty &&
        current.baseUrl.trim().isNotEmpty &&
        current.model.trim().isNotEmpty;

    final accepted = await showDialog<bool>(
      context: context,
      barrierColor: const Color(0x520f172a),
      builder: (dialogContext) => StatefulBuilder(
        builder: (dialogContext, update) {
          void validate() => update(() {
            valid =
                name.text.trim().isNotEmpty &&
                base.text.trim().isNotEmpty &&
                model.text.trim().isNotEmpty;
          });
          Widget field(
            String label,
            TextEditingController controller, {
            bool obscure = false,
            String? hint,
          }) => Padding(
            padding: const EdgeInsets.only(bottom: 14),
            child: Row(
              children: [
                SizedBox(
                  width: 120,
                  child: Text(
                    label,
                    style: const TextStyle(
                      fontSize: 13,
                      color: Color(0xff4e5566),
                    ),
                  ),
                ),
                const SizedBox(width: 14),
                Expanded(
                  child: SizedBox(
                    height: 32,
                    child: TextField(
                      controller: controller,
                      obscureText: obscure,
                      textAlignVertical: TextAlignVertical.center,
                      onChanged: (_) => validate(),
                      style: const TextStyle(
                        fontSize: 13,
                        height: 1.2,
                        color: Color(0xff1f2329),
                      ),
                      decoration: InputDecoration(
                        hintText: hint,
                        constraints: const BoxConstraints.tightFor(height: 32),
                        isDense: true,
                        contentPadding: const EdgeInsets.symmetric(
                          horizontal: 10,
                          vertical: 7,
                        ),
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8),
                          borderSide: const BorderSide(
                            color: Color(0xffe2e4e8),
                          ),
                        ),
                        enabledBorder: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8),
                          borderSide: const BorderSide(
                            color: Color(0xffe2e4e8),
                          ),
                        ),
                        focusedBorder: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8),
                          borderSide: BorderSide(color: widget.config.color),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          );

          return Dialog(
            key: const ValueKey('model-editor-dialog'),
            elevation: 0,
            backgroundColor: Colors.transparent,
            insetPadding: const EdgeInsets.all(24),
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(14),
            ),
            child: Container(
              key: const ValueKey('model-editor-surface'),
              width: 460,
              constraints: const BoxConstraints(maxHeight: 592),
              decoration: BoxDecoration(
                color: Colors.white,
                borderRadius: BorderRadius.circular(14),
                boxShadow: const [
                  BoxShadow(
                    color: Color(0x2e0f172a),
                    blurRadius: 48,
                    offset: Offset(0, 18),
                  ),
                ],
              ),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 22,
                  vertical: 20,
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      current == null ? '添加模型' : '编辑模型',
                      style: const TextStyle(
                        fontSize: 15,
                        height: 1.2,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                    const SizedBox(height: 14),
                    Flexible(
                      child: SingleChildScrollView(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            if (current == null) ...[
                              Padding(
                                padding: const EdgeInsets.only(bottom: 14),
                                child: Row(
                                  children: [
                                    const SizedBox(
                                      width: 120,
                                      child: Text(
                                        '快速预设',
                                        style: TextStyle(
                                          fontSize: 13,
                                          color: Color(0xff4e5566),
                                        ),
                                      ),
                                    ),
                                    const SizedBox(width: 14),
                                    Expanded(
                                      child: SizedBox(
                                        height: 32,
                                        child: DropdownButtonFormField<String>(
                                          isExpanded: true,
                                          decoration: InputDecoration(
                                            isDense: true,
                                            contentPadding:
                                                const EdgeInsets.symmetric(
                                                  horizontal: 10,
                                                ),
                                            border: OutlineInputBorder(
                                              borderRadius:
                                                  BorderRadius.circular(8),
                                            ),
                                          ),
                                          hint: const Text(
                                            '选择预设（可改）…',
                                            style: TextStyle(fontSize: 13),
                                            overflow: TextOverflow.ellipsis,
                                          ),
                                          items: [
                                            for (final preset in _presets)
                                              DropdownMenuItem(
                                                value: preset.$1,
                                                child: Text(
                                                  '${preset.$1} · ${preset.$2}',
                                                  style: const TextStyle(
                                                    fontSize: 13,
                                                  ),
                                                  overflow:
                                                      TextOverflow.ellipsis,
                                                ),
                                              ),
                                          ],
                                          onChanged: (value) {
                                            final preset = _presets.firstWhere(
                                              (item) => item.$1 == value,
                                            );
                                            name.text = preset.$1 == '自定义'
                                                ? ''
                                                : preset.$1;
                                            note.text = preset.$2;
                                            if (preset.$3.isNotEmpty)
                                              base.text = preset.$3;
                                            if (preset.$4.isNotEmpty)
                                              model.text = preset.$4;
                                            if (preset.$5.isNotEmpty)
                                              keyUrl.text = preset.$5;
                                            validate();
                                          },
                                        ),
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                            ],
                            field('名称', name, hint: 'DeepSeek-V4 Pro'),
                            field('备注', note, hint: '深度搜索'),
                            field(
                              'Base URL',
                              base,
                              hint: 'https://api.deepseek.com',
                            ),
                            field('API Key', key, obscure: true, hint: 'sk-…'),
                            field('模型 ID', model, hint: 'deepseek-flash'),
                            field(
                              '获取 Key 网址',
                              keyUrl,
                              hint: 'https://platform.deepseek.com/api_keys',
                            ),
                          ],
                        ),
                      ),
                    ),
                    const SizedBox(height: 4),
                    Row(
                      mainAxisAlignment: MainAxisAlignment.end,
                      children: [
                        OutlinedButton(
                          onPressed: () => Navigator.pop(dialogContext, false),
                          child: const Text('取消'),
                        ),
                        const SizedBox(width: 10),
                        FilledButton(
                          onPressed: valid
                              ? () => Navigator.pop(dialogContext, true)
                              : null,
                          child: const Text('确定'),
                        ),
                      ],
                    ),
                  ],
                ),
              ),
            ),
          );
        },
      ),
    );

    AppLog.write(
      'model-editor-result',
      accepted == true ? 'confirm' : 'cancel',
    );
    if (accepted == true && mounted) {
      final updated = TranslationModel(
        id: current?.id ?? 'm${DateTime.now().microsecondsSinceEpoch}',
        name: name.text,
        note: note.text,
        baseUrl: base.text,
        apiKey: key.text,
        model: model.text,
        keyUrl: keyUrl.text,
      );
      final models = [...widget.config.translationModels];
      final index = models.indexWhere((entry) => entry.id == updated.id);
      if (index < 0) {
        models.add(updated);
      } else {
        models[index] = updated;
      }
      widget.onChanged(widget.config.copyWith(translationModels: models));
    }
    await Future<void>.delayed(const Duration(milliseconds: 220));
    for (final controller in [name, note, base, key, model, keyUrl]) {
      controller.dispose();
    }
  }

  Future<void> _test(TranslationModel model) async {
    setState(() {
      _testing = model.id;
      _result = null;
    });
    try {
      final saved = widget.savedConfig ?? await ConfigStore.load();
      final candidates = saved.translationModels.where(
        (entry) => entry.id == model.id,
      );
      if (candidates.isEmpty) throw const FormatException('找不到这个模型（可能已被删除）');
      AppLog.write('model-connection-test-start');
      await TranslationService.testConnection(candidates.first);
      AppLog.write('model-connection-test-success');
      if (mounted)
        setState(() {
          _testOk = true;
          _result = '连接成功：端点与 API Key 均可用';
        });
    } catch (error) {
      AppLog.write(
        'model-connection-test-failed',
        error is FormatException ? error.message : error.runtimeType,
      );
      if (mounted)
        setState(() {
          _testOk = false;
          _result = error is FormatException
              ? error.message.toString()
              : error.toString();
        });
    } finally {
      if (mounted) setState(() => _testing = null);
    }
  }

  Future<void> _openKeyUrl(String url) async {
    try {
      await Process.start(Platform.isMacOS ? 'open' : 'explorer.exe', [url]);
    } catch (error) {
      AppLog.write('model-key-url-open-failed', error.runtimeType);
    }
  }

  @override
  Widget build(BuildContext context) {
    final models = widget.config.translationModels;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    '自定义模型',
                    style: TextStyle(
                      fontSize: 15,
                      height: 1.2,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  SizedBox(height: 6),
                  SizedBox(
                    width: 420,
                    height: 38.4,
                    child: Text(
                      '模型写入本地 config.json，API Key 只存在本机；列表第一个为当前启用的模型。',
                      style: TextStyle(
                        fontSize: 12,
                        height: 1.6,
                        color: Color(0xff8a919f),
                      ),
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 16),
            OutlinedButton(onPressed: () => _edit(), child: const Text('添加模型')),
          ],
        ),
        const SizedBox(height: 16),
        Container(
          width: double.infinity,
          key: const ValueKey('model-list-card'),
          padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 18),
          decoration: BoxDecoration(
            border: Border.all(color: const Color(0xffeceef1)),
            borderRadius: BorderRadius.circular(12),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (models.isEmpty)
                const Padding(
                  padding: EdgeInsets.symmetric(vertical: 4),
                  child: Text(
                    '还没有模型。点右上角「添加模型」，或从预设里选一个，只剩 API Key 需要你填。',
                    style: TextStyle(fontSize: 12, color: Color(0xff9aa0ab)),
                  ),
                ),
              for (var index = 0; index < models.length; index++)
                _modelRow(
                  models[index],
                  index == 0,
                  last: index == models.length - 1,
                ),
              if (_result != null && _testing == null)
                Container(
                  width: double.infinity,
                  margin: const EdgeInsets.only(top: 8),
                  padding: const EdgeInsets.symmetric(
                    horizontal: 12,
                    vertical: 8,
                  ),
                  decoration: BoxDecoration(
                    color: _testOk
                        ? const Color(0xffedf9ee)
                        : const Color(0xfffdecec),
                    borderRadius: BorderRadius.circular(8),
                  ),
                  child: Text(
                    _result!,
                    style: TextStyle(
                      fontSize: 12,
                      color: _testOk
                          ? const Color(0xff2a7d32)
                          : const Color(0xffc0392b),
                    ),
                  ),
                ),
              if (models.length > 1)
                const Padding(
                  padding: EdgeInsets.only(top: 12),
                  child: Text(
                    '想切换启用模型：把更常用的那个删掉再重新添加，它就会排到最前面（列表第一个 = 当前启用）。',
                    style: TextStyle(fontSize: 12, color: Color(0xff9aa0ab)),
                  ),
                ),
            ],
          ),
        ),
      ],
    );
  }

  Widget _modelRow(
    TranslationModel model,
    bool active, {
    bool last = false,
  }) => MouseRegion(
    onEnter: (_) => setState(() => _hovered = model.id),
    onExit: (_) => setState(() => _hovered = null),
    child: Focus(
      onFocusChange: (focused) =>
          setState(() => _focused = focused ? model.id : null),
      child: Container(
        key: ValueKey('model-row-${model.id}'),
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 12),
        decoration: BoxDecoration(
          border: last
              ? null
              : const Border(bottom: BorderSide(color: Color(0xfff5f6f7))),
        ),
        child: Row(
          children: [
            Container(
              width: 28,
              height: 28,
              alignment: Alignment.center,
              decoration: BoxDecoration(
                color: const Color(0xffeef0f3),
                borderRadius: BorderRadius.circular(8),
              ),
              child: Text(
                model.name.isEmpty ? '?' : model.name.substring(0, 1),
                style: const TextStyle(
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                  color: Color(0xff4e5566),
                ),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SizedBox(
                    height: 16,
                    child: Row(
                      children: [
                        Flexible(
                          child: Text(
                            model.name.isEmpty ? '（未命名）' : model.name,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(fontSize: 13, height: 1.2),
                          ),
                        ),
                        if (active) ...[
                          const SizedBox(width: 6),
                          Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 6,
                              vertical: 1,
                            ),
                            decoration: BoxDecoration(
                              color: const Color(0xffeaf6ff),
                              borderRadius: BorderRadius.circular(4),
                            ),
                            child: const Text(
                              '使用中',
                              style: TextStyle(
                                fontSize: 11,
                                height: 1.2,
                                color: Color(0xff1677ff),
                              ),
                            ),
                          ),
                        ],
                      ],
                    ),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    [
                          model.note,
                          model.model,
                        ].where((value) => value.isNotEmpty).join(' · ').isEmpty
                        ? model.baseUrl
                        : [
                            model.note,
                            model.model,
                          ].where((value) => value.isNotEmpty).join(' · '),
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(
                      fontSize: 12,
                      height: 1.2,
                      color: Color(0xff9aa0ab),
                    ),
                  ),
                ],
              ),
            ),
            IgnorePointer(
              ignoring:
                  _hovered != model.id &&
                  _focused != model.id &&
                  _testing != model.id,
              child: AnimatedOpacity(
                opacity:
                    _hovered == model.id ||
                        _focused == model.id ||
                        _testing == model.id
                    ? 1
                    : 0,
                duration: const Duration(milliseconds: 120),
                child: TextButton(
                  onPressed: _testing == null ? () => _test(model) : null,
                  style: TextButton.styleFrom(
                    backgroundColor: const Color(0xff1f2329),
                    foregroundColor: Colors.white,
                    minimumSize: const Size(0, 26),
                    padding: const EdgeInsets.symmetric(horizontal: 10),
                  ),
                  child: Text(
                    _testing != null ? '测试中…' : '测试连接',
                    style: const TextStyle(fontSize: 12),
                  ),
                ),
              ),
            ),
            const SizedBox(width: 2),
            IconButton(
              visualDensity: VisualDensity.standard,
              style: const ButtonStyle(
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
              ),
              tooltip: '编辑',
              onPressed: () => _edit(model),
              constraints: const BoxConstraints.tightFor(width: 26, height: 26),
              padding: EdgeInsets.zero,
              icon: settingsIcon('pencil', size: 15),
            ),
            const SizedBox(width: 2),
            IconButton(
              visualDensity: VisualDensity.standard,
              style: const ButtonStyle(
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
              ),
              tooltip: '获取 API Key 的网页',
              onPressed: model.keyUrl.isEmpty
                  ? null
                  : () => _openKeyUrl(model.keyUrl),
              constraints: const BoxConstraints.tightFor(width: 26, height: 26),
              padding: EdgeInsets.zero,
              icon: settingsIcon(
                'link',
                size: 15,
                color: model.keyUrl.isEmpty
                    ? const Color(0x668a919f)
                    : const Color(0xff8a919f),
              ),
            ),
            const SizedBox(width: 2),
            IconButton(
              visualDensity: VisualDensity.standard,
              style: const ButtonStyle(
                tapTargetSize: MaterialTapTargetSize.shrinkWrap,
              ),
              hoverColor: const Color(0xfffdecec),
              tooltip: '删除',
              onPressed: () => widget.onChanged(
                widget.config.copyWith(
                  translationModels: modelsWithout(model.id),
                ),
              ),
              constraints: const BoxConstraints.tightFor(width: 26, height: 26),
              padding: EdgeInsets.zero,
              icon: settingsIcon('trash', size: 15),
            ),
          ],
        ),
      ),
    ),
  );

  List<TranslationModel> modelsWithout(String id) =>
      widget.config.translationModels.where((model) => model.id != id).toList();
}
