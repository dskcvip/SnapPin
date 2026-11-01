"""Check architectures, deployment targets and engine resources before launch."""
import json
import pathlib
import plistlib
import re
import subprocess
import sys

app = pathlib.Path(sys.argv[1]).resolve()
report = pathlib.Path(sys.argv[2])
report.parent.mkdir(parents=True, exist_ok=True)
info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
assert info.get('FLTEnableImpeller') is False, 'Skia compatibility setting missing'
assert tuple(map(int, info['LSMinimumSystemVersion'].split('.'))) <= (12, 0)
frameworks = app / 'Contents/Frameworks'
assert (frameworks / 'App.framework/Resources/flutter_assets/AssetManifest.bin').is_file()
assert (frameworks / 'FlutterMacOS.framework/Resources/icudtl.dat').is_file()
files = [app / 'Contents/MacOS/SnapPin', *frameworks.rglob('*')]
records = []
seen = set()
for file in files:
    if not file.is_file() or file.resolve() in seen:
        continue
    seen.add(file.resolve())
    if 'Mach-O' not in subprocess.check_output(['file', '-b', str(file)], text=True):
        continue
    record = {'path': str(file.relative_to(app)), 'architectures': {}}
    for architecture in ['x86_64', 'arm64']:
        subprocess.run(['lipo', str(file), '-verify_arch', architecture], check=True)
        commands = subprocess.check_output(['otool', '-arch', architecture, '-l', str(file)], text=True)
        targets = re.findall(r'\bminos\s+(\d+(?:\.\d+)+)', commands)
        targets += re.findall(r'LC_VERSION_MIN_MACOSX\s+cmdsize\s+\d+\s+version\s+(\d+(?:\.\d+)+)', commands)
        assert targets, f'No deployment target: {file} ({architecture})'
        for target in targets:
            major, minor, *_ = map(int, target.split('.'))
            assert (major, minor) <= (12, 0), f'{file} ({architecture}) requires macOS {target}'
        record['architectures'][architecture] = targets
    records.append(record)
assert len(records) >= 3, 'Missing app or Flutter binaries'
report.write_text(json.dumps({'renderer': 'Skia/Metal', 'minimum_os': info['LSMinimumSystemVersion'],
                              'binaries': records}, indent=2))
print(f'Bundle architecture/deployment/resource checks passed: {len(records)} binaries')
