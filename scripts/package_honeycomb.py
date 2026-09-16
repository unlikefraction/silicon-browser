#!/usr/bin/env python3
"""Stage native-tested binaries, then pack all six targets using only the stdlib.

Stage on each native host: --stage TARGET --binary PATH [--controller PATH]
Pack downloaded target artifacts: --targets DIRECTORY --output ARCHIVE.tar.gz
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tarfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
CONTROLLER_VERSION = "0.36.0"
CONTROLLER_COMMIT = "eb05921bad874cd2a1b4fa5d1149f1ed26576cae"
TARGETS = {
    "linux-x86_64": "agent-browser-linux-x64",
    "linux-aarch64": "agent-browser-linux-arm64",
    "windows-x86_64": "agent-browser-win32-x64.exe",
    "windows-aarch64": None,  # No upstream binary; build the pinned source natively.
    "macos-x86_64": "agent-browser-darwin-x64",
    "macos-aarch64": "agent-browser-darwin-arm64",
}
LICENSES = {
    "LICENSE-agent-browser": "LICENSE",
    "LICENSE-axe-core.txt": "cli/src/native/a11y/LICENSE-axe-core.txt",
    "LICENSE-axe-core-THIRD-PARTY.txt": "cli/src/native/a11y/LICENSE-axe-core-THIRD-PARTY.txt",
}


def version():
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def binaries(target):
    suffix = ".exe" if target.startswith("windows-") else ""
    return ["sb" + suffix, "sb-browser-engine" + suffix]


def native_target(data):
    """Reject scripts and accidental copies of another platform's executable."""
    if data[:6] == b"\x7fELF\x02\x01" and len(data) >= 64:
        machine = struct.unpack_from("<H", data, 18)[0]
        return {62: "linux-x86_64", 183: "linux-aarch64"}.get(machine)
    if data[:4] == b"\xcf\xfa\xed\xfe" and len(data) >= 32:
        machine = struct.unpack_from("<I", data, 4)[0]
        return {0x1000007: "macos-x86_64", 0x100000C: "macos-aarch64"}.get(machine)
    if data[:2] == b"MZ" and len(data) >= 64:
        offset = struct.unpack_from("<I", data, 60)[0]
        if offset + 6 <= len(data) and data[offset:offset + 4] == b"PE\0\0":
            machine = struct.unpack_from("<H", data, offset + 4)[0]
            return {0x8664: "windows-x86_64", 0xAA64: "windows-aarch64"}.get(machine)
    return None


def regular_bytes(path):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f"missing regular file: {path}")
    return path.read_bytes()


def download(url):
    with urllib.request.urlopen(url, timeout=120) as response:
        data = response.read(64 * 1024 * 1024 + 1)
    if len(data) > 64 * 1024 * 1024:
        raise ValueError(f"download exceeds 64 MiB: {url}")
    return data


def stage(target, binary, controller, directory):
    directory.mkdir(parents=True, exist_ok=True)
    names = binaries(target)
    shutil.copyfile(binary, directory / names[0])
    if controller:
        shutil.copyfile(controller, directory / names[1])
    else:
        asset = TARGETS[target]
        if asset is None:
            raise ValueError("Windows ARM64 requires --controller built from the pinned source")
        data = download(f"https://github.com/vercel-labs/agent-browser/releases/download/v{CONTROLLER_VERSION}/{asset}")
        # Keep the download integrity pins in the existing Rust setup code.
        source = (ROOT / "crates/client/src/setup.rs").read_text()
        digest = re.search(r'"' + re.escape(asset) + r'",\s*"([a-f0-9]{64})"', source)
        if digest is None or hashlib.sha256(data).hexdigest() != digest[1]:
            raise ValueError(f"controller integrity check failed: {asset}")
        (directory / names[1]).write_bytes(data)
    for name, expected in zip(names, (version(), CONTROLLER_VERSION)):
        path = directory / name
        if native_target(regular_bytes(path)) != target:
            raise ValueError(f"wrong native architecture: {path}")
        path.chmod(0o755)
        output = subprocess.check_output([str(path.resolve()), "--version"], text=True).strip()
        if output.split()[-1:] != [expected]:
            raise ValueError(f"wrong version for {path}: {output}")
        subprocess.run([str(path.resolve()), "--help"], check=True, stdout=subprocess.DEVNULL)
    shutil.copyfile(ROOT / "LICENSE", directory / "LICENSE-silicon-browser")
    for name, source in LICENSES.items():
        (directory / name).write_bytes(download(
            f"https://raw.githubusercontent.com/vercel-labs/agent-browser/{CONTROLLER_COMMIT}/{source}"))
    # The receipt ties versions checked on the native host to the exact packaged bytes.
    receipt = {"version": version(), "controller_version": CONTROLLER_VERSION,
               "sha256": {name: hashlib.sha256(regular_bytes(directory / name)).hexdigest() for name in names}}
    (directory / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")


def pack(targets, output):
    manifest = regular_bytes(ROOT / "honeycomb.yaml")
    if re.findall(rb"^version: (.+)$", manifest, re.MULTILINE) != [version().encode()]:
        raise ValueError("honeycomb.yaml version does not match Cargo.toml")
    files = [("honeycomb.yaml", manifest, 0o644)]
    for target in TARGETS:
        directory = targets / target
        if directory.is_symlink() or not directory.is_dir():
            raise ValueError(f"missing regular target directory: {directory}")
        receipt = json.loads(regular_bytes(directory / "build.json"))
        if (receipt.get("version"), receipt.get("controller_version")) != (version(), CONTROLLER_VERSION):
            raise ValueError(f"wrong version receipt: {target}")
        names = binaries(target)
        for name in [*names, "LICENSE-silicon-browser", *LICENSES]:
            data = regular_bytes(directory / name)
            if name in names:
                if native_target(data) != target:
                    raise ValueError(f"wrong native architecture: {target}/{name}")
                if receipt.get("sha256", {}).get(name) != hashlib.sha256(data).hexdigest():
                    raise ValueError(f"binary differs from native-tested receipt: {target}/{name}")
            # Explicit allowlist: neither source trees, state nor receipts enter the archive.
            files.append((f"targets/{target}/{name}", data, 0o755 if name in names else 0o644))
    if sum(len(data) for _, data, _ in files) > 2 * 1024**3:
        raise ValueError("expanded package exceeds Honeycomb's 2 GiB limit")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tarfile.open(output, "w:gz", format=tarfile.USTAR_FORMAT) as archive:
        for name, data, mode in files:
            entry = tarfile.TarInfo(name)
            entry.size, entry.mode = len(data), mode
            archive.addfile(entry, io.BytesIO(data))
    if output.stat().st_size > 512 * 1024**2:
        output.unlink()
        raise ValueError("compressed package exceeds Honeycomb's 512 MiB limit")
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--targets", type=Path, default=ROOT / "target/honeycomb/targets")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--stage", choices=TARGETS)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--controller", type=Path)
    args = parser.parse_args()
    if args.stage:
        if not args.binary:
            parser.error("--stage requires --binary")
        stage(args.stage, args.binary, args.controller, args.targets / args.stage)
    else:
        pack(args.targets, args.output or ROOT / f"target/honeycomb-browser-{version()}.tar.gz")
