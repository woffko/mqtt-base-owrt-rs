#!/usr/bin/env python3
"""Assemble and verify the release assets from tools/release-build.sh output.

Usage: release-assets.py <work-dir> <asset-dir>

For every architecture in <work-dir>/out/ the daemon package is copied as
mqtt-metrics_<version>_<arch>.apk, the LuCI package once as
luci-app-mqtt-metrics_<version>_all.apk. Every package is checked: name,
version and architecture in its metadata, the declared dependencies, and the
ELF machine of /usr/sbin/mqtt-metricsd against the architecture. SHA256SUMS
and manifest.json are written next to the packages. Any mismatch stops with
an error and nothing is left half-written.
"""

import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# ELF machine and class (bits) per package architecture prefix.
ELF = {
    'x86_64': ('Advanced Micro Devices X86-64', 64),
    'aarch64': ('AArch64', 64),
    'arm_': ('ARM', 32),
    'mips_': ('MIPS R3000', 32),
    'mipsel_': ('MIPS R3000', 32),
}
DAEMON_DEPENDS = {'ca-bundle', 'libc', 'libgcc1', 'ubus', 'uci'}
LUCI_DEPENDS = {'libc', 'luci-base', 'mqtt-metrics'}


def makefile_version(package):
    text = (REPO / 'package' / package / 'Makefile').read_text()
    version = re.search(r'^PKG_VERSION:=(\S+)', text, re.M).group(1)
    release = re.search(r'^PKG_RELEASE:=(\S+)', text, re.M).group(1)
    return f'{version}-r{release}'


def apk_tool(work):
    if os.environ.get('APK'):
        return os.environ['APK']
    tools = sorted(work.glob('sdk/*/staging_dir/host/bin/apk'))
    if not tools:
        sys.exit('no apk tool in any SDK under ' + str(work))
    return str(tools[0])


def metadata(apk, path):
    dump = subprocess.run([apk, 'adbdump', '--allow-untrusted', str(path)],
                          check=True, capture_output=True, text=True).stdout
    info = {}
    for key in ('name', 'version', 'arch', 'license'):
        m = re.search(rf'^  {key}: (.+)$', dump, re.M)
        info[key] = m.group(1).strip() if m else None
    block = re.search(r'^  depends: #.*\n((?:    - .*\n)*)', dump, re.M)
    info['depends'] = sorted(re.findall(r'    - (\S+)', block.group(1))) if block else []
    return info


def elf_of(apk, path):
    with tempfile.TemporaryDirectory() as tmp:
        subprocess.run([apk, 'extract', '--allow-untrusted', '--destination', tmp, str(path)],
                       check=True, capture_output=True)
        binary = Path(tmp) / 'usr/sbin/mqtt-metricsd'
        header = subprocess.run(['readelf', '-h', str(binary)], check=True,
                                capture_output=True, text=True).stdout
        machine = re.search(r'Machine:\s+(.+)', header).group(1).strip()
        bits = 64 if 'ELF64' in header else 32
        tls = b'rustls' in binary.read_bytes()
        return machine, bits, tls


def sha256(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for chunk in iter(lambda: f.read(1 << 20), b''):
            h.update(chunk)
    return h.hexdigest()


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    work, dest = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    apk = apk_tool(work)
    daemon_version = makefile_version('mqtt-metrics')
    luci_version = makefile_version('luci-app-mqtt-metrics')
    staging = Path(tempfile.mkdtemp(prefix='release-assets-', dir=dest.parent))
    manifest = {'mqtt-metrics': daemon_version, 'luci-app-mqtt-metrics': luci_version, 'packages': []}
    try:
        luci = None
        for arch_dir in sorted((work / 'out').iterdir()):
            arch = arch_dir.name
            source = arch_dir / f'mqtt-metrics-{daemon_version}.apk'
            if not source.is_file():
                sys.exit(f'{arch}: {source.name} is missing')
            info = metadata(apk, source)
            assert info['name'] == 'mqtt-metrics', (arch, info)
            assert info['version'] == daemon_version, (arch, info)
            assert info['arch'] == arch, (arch, info)
            assert set(info['depends']) == DAEMON_DEPENDS, (arch, info['depends'])
            machine, bits, tls = elf_of(apk, source)
            expected = next(v for k, v in ELF.items() if arch.startswith(k))
            assert (machine, bits) == expected, (arch, machine, bits)
            assert tls, f'{arch}: the binary has no TLS support'
            name = f'mqtt-metrics_{daemon_version}_{arch}.apk'
            shutil.copy2(source, staging / name)
            manifest['packages'].append({'file': name, 'arch': arch, 'elf': f'{machine}, {bits}-bit',
                                         'depends': info['depends'], 'sha256': sha256(source)})
            print(f'{arch}: {name} ({machine}, {bits}-bit, TLS)')
            candidate = arch_dir / f'luci-app-mqtt-metrics-{luci_version}.apk'
            if candidate.is_file():
                luci = candidate
        if luci is None:
            sys.exit(f'luci-app-mqtt-metrics-{luci_version}.apk was not built')
        info = metadata(apk, luci)
        assert info['name'] == 'luci-app-mqtt-metrics' and info['version'] == luci_version, info
        assert info['arch'] == 'noarch', info
        assert set(info['depends']) == LUCI_DEPENDS, info['depends']
        name = f'luci-app-mqtt-metrics_{luci_version}_all.apk'
        shutil.copy2(luci, staging / name)
        manifest['packages'].append({'file': name, 'arch': 'all', 'depends': info['depends'],
                                     'sha256': sha256(luci)})
        print(f'all: {name}')
        files = sorted(p.name for p in staging.glob('*.apk'))
        (staging / 'SHA256SUMS').write_text(''.join(f'{sha256(staging / f)}  {f}\n' for f in files))
        (staging / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
        if dest.exists():
            shutil.rmtree(dest)
        staging.rename(dest)
        print(f'{len(files)} packages in {dest}')
    finally:
        if staging.exists():
            shutil.rmtree(staging)


if __name__ == '__main__':
    main()
