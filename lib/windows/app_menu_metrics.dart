const appMenuWidth = 260.0;
const appMenuBaseHeight = 300.0;
const appMenuWarningHeight = 46.0;

double appMenuHeight(List<String> warnings, {bool macOS = false}) =>
    appMenuBaseHeight +
    (warnings.isEmpty ? 0 : appMenuWarningHeight) +
    (macOS ? 32 : 0);

String appMenuWarningText(List<String> warnings) =>
    warnings.any((warning) => warning.contains('配置无效'))
    ? warnings.join(' / ')
    : '${warnings.join(' / ')} 已被其他程序占用';
