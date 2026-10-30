import 'dart:io';

import 'package:flutter/material.dart';

import '../app_config.dart';
import '../translation_service.dart';

class ModelsSettings extends StatefulWidget {
  const ModelsSettings({
    super.key,
    required this.config,
    required this.onChanged,
  });

  final AppConfig config;
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
  String? _testing;
  String? _result;
  bool _testOk = false;

  Future<void> _edit([TranslationModel? current]) async {
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
                      onChanged: (_) => validate(),
                      style: const TextStyle(fontSize: 13),
                      decoration: InputDecoration(
                        hintText: hint,
                        isDense: true,
                        contentPadding: const EdgeInsets.symmetric(
                          horizontal: 10,
                        ),
                        border: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(8),
                        ),
                      ),
                    ),
                  ),
                ),
              ],
            ),
          );

          return Dialog(
            backgroundColor: Colors.white,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(14),
            ),
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 460, maxHeight: 520),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(22, 20, 22, 18),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      current == null ? '添加模型' : '编辑模型',
                      style: const TextStyle(
                        fontSize: 15,
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
                        const SizedBox(width: 8),
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

    if (accepted == true) {
      final updated = TranslationModel(
        id: current?.id ?? 'm${DateTime.now().microsecondsSinceEpoch}',
        name: name.text.trim(),
        note: note.text.trim(),
        baseUrl: base.text.trim(),
        apiKey: key.text,
        model: model.text.trim(),
        keyUrl: keyUrl.text.trim(),
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
      await TranslationService.translate('你好', model, 'English');
      if (mounted)
        setState(() {
          _testOk = true;
          _result = '连接成功：端点与 API Key 均可用';
        });
    } catch (error) {
      if (mounted)
        setState(() {
          _testOk = false;
          _result = error.toString();
        });
    } finally {
      if (mounted) setState(() => _testing = null);
    }
  }

  Future<void> _openKeyUrl(String url) async {
    await Process.start(Platform.isMacOS ? 'open' : 'explorer.exe', [url]);
  }

  @override
  Widget build(BuildContext context) {
    final models = widget.config.translationModels;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            const Expanded(
              child: Text(
                '自定义模型',
                style: TextStyle(fontSize: 15, fontWeight: FontWeight.w600),
              ),
            ),
            OutlinedButton(onPressed: () => _edit(), child: const Text('添加模型')),
          ],
        ),
        const SizedBox(height: 6),
        const Text(
          '模型写入本地 config.json，API Key 只存在本机；列表第一个为当前启用的模型。',
          style: TextStyle(fontSize: 12, color: Color(0xff8a919f)),
        ),
        const SizedBox(height: 16),
        Container(
          width: double.infinity,
          padding: const EdgeInsets.symmetric(horizontal: 20, vertical: 8),
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
                _modelRow(models[index], index == 0),
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

  Widget _modelRow(TranslationModel model, bool active) => MouseRegion(
    onEnter: (_) => setState(() => _hovered = model.id),
    onExit: (_) => setState(() => _hovered = null),
    child: Container(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 12),
      decoration: const BoxDecoration(
        border: Border(bottom: BorderSide(color: Color(0xfff5f6f7))),
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
                Row(
                  children: [
                    Flexible(
                      child: Text(
                        model.name.isEmpty ? '（未命名）' : model.name,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(fontSize: 13),
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
                            color: Color(0xff1677ff),
                          ),
                        ),
                      ),
                    ],
                  ],
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
                    color: Color(0xff9aa0ab),
                  ),
                ),
              ],
            ),
          ),
          AnimatedOpacity(
            opacity: _hovered == model.id || _testing == model.id ? 1 : 0,
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
                _testing == model.id ? '测试中…' : '测试连接',
                style: const TextStyle(fontSize: 12),
              ),
            ),
          ),
          IconButton(
            tooltip: '编辑',
            onPressed: () => _edit(model),
            icon: const Icon(Icons.edit_outlined, size: 16),
          ),
          IconButton(
            tooltip: '获取 API Key 的网页',
            onPressed: model.keyUrl.isEmpty
                ? null
                : () => _openKeyUrl(model.keyUrl),
            icon: const Icon(Icons.link, size: 16),
          ),
          IconButton(
            tooltip: '删除',
            onPressed: () => widget.onChanged(
              widget.config.copyWith(
                translationModels: modelsWithout(model.id),
              ),
            ),
            icon: const Icon(Icons.delete_outline, size: 16),
          ),
        ],
      ),
    ),
  );

  List<TranslationModel> modelsWithout(String id) =>
      widget.config.translationModels.where((model) => model.id != id).toList();
}
