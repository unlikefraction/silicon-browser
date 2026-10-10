#!/usr/bin/env python3
"""Stage native-tested binaries, then pack all six targets using only the stdlib.

Stage on each native host: --stage TARGET --binary PATH [--controller PATH]
Pack downloaded target artifacts: --targets DIRECTORY --output ARCHIVE.tar.gz
Also build the curl installer archives: --standalone-output DIRECTORY
"""
import argparse
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tarfile
import tempfile
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
STANDALONE_TARGETS = {
    "linux-x86_64": "x86_64-unknown-linux-gnu",
    "linux-aarch64": "aarch64-unknown-linux-gnu",
    "macos-x86_64": "x86_64-apple-darwin",
    "macos-aarch64": "aarch64-apple-darwin",
}
LICENSES = {
    "LICENSE-agent-browser": "LICENSE",
    "LICENSE-axe-core.txt": "cli/src/native/a11y/LICENSE-axe-core.txt",
    "LICENSE-axe-core-THIRD-PARTY.txt": "cli/src/native/a11y/LICENSE-axe-core-THIRD-PARTY.txt",
}
MAX_EXPANDED_SIZE = 1024**3
MAX_COMPRESSED_SIZE = 512 * 1024**2
MAX_ENTRIES = 20_000


def version():
    value = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if not re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)", value):
        raise ValueError("Silicon Apps version must be strict x.y.z without prerelease or build metadata")
    return value


def binaries(target):
    suffix = ".exe" if target.startswith("windows-") else ""
    return ["browser" + suffix, "sb-browser-engine" + suffix]


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
    with tempfile.TemporaryDirectory(prefix="browser-package-check-") as home:
        env = {key: value for key, value in os.environ.items() if not key.startswith("SB_")}
        env.update(SB_HOME=home, SB_BACKEND_URL="http://127.0.0.1:1")
        for name, expected in zip(names, (version(), CONTROLLER_VERSION)):
            path = directory / name
            if native_target(regular_bytes(path)) != target:
                raise ValueError(f"wrong native architecture: {path}")
            path.chmod(0o755)
            output = subprocess.check_output([str(path.resolve()), "--version"], text=True, env=env).strip()
            if output.split()[-1:] != [expected] or (name == names[0] and output != f"browser {expected}"):
                raise ValueError(f"wrong version for {path}: {output}")
            subprocess.run([str(path.resolve()), "--help"], check=True, stdout=subprocess.DEVNULL, env=env)
            if name == names[0]:
                for arguments, field, expected_value in [
                    (["accounts", "--json"], "app_id", "browser"),
                    (["login", "status", "--json"], "authenticated", False),
                ]:
                    result = json.loads(subprocess.check_output([str(path.resolve()), *arguments], text=True, env=env))
                    if result.get(field) != expected_value or type(result.get(field)) is not type(expected_value):
                        raise ValueError(f"invalid discovery command for {path}: {' '.join(arguments)}")
    shutil.copyfile(ROOT / "LICENSE", directory / "LICENSE-silicon-browser")
    for name, source in LICENSES.items():
        (directory / name).write_bytes(download(
            f"https://raw.githubusercontent.com/vercel-labs/agent-browser/{CONTROLLER_COMMIT}/{source}"))
    # The receipt ties versions checked on the native host to the exact packaged bytes.
    receipt = {"version": version(), "controller_version": CONTROLLER_VERSION,
               "sha256": {name: hashlib.sha256(regular_bytes(directory / name)).hexdigest() for name in names}}
    (directory / "build.json").write_text(json.dumps(receipt, indent=2) + "\n")


def write_archive(output, files):
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("wb") as destination, gzip.GzipFile(
        fileobj=destination, filename="", mode="wb", mtime=0
    ) as compressed, tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
        for name, data, mode in files:
            entry = tarfile.TarInfo(name)
            entry.size, entry.mode = len(data), mode
            archive.addfile(entry, io.BytesIO(data))


def pack(targets, output, standalone_output=None):
    manifest = regular_bytes(ROOT / "apps.yaml")
    if re.findall(rb"^version: (.+)$", manifest, re.MULTILINE) != [version().encode()]:
        raise ValueError("apps.yaml version does not match Cargo.toml")
    if re.findall(rb"^app_id: (.+)$", manifest, re.MULTILINE) != [b"browser"]:
        raise ValueError("apps.yaml app_id must be browser")
    expected_manifest = ["schema_version: 1", "app_id: browser", f"version: {version()}", "command: browser", "targets:"]
    for target in TARGETS:
        expected_manifest.extend([f"  {target}:", f"    binary: targets/{target}/{binaries(target)[0]}"])
    if manifest.decode().splitlines() != expected_manifest:
        raise ValueError("apps.yaml must use schema_version 1, command browser and the native target binary paths")
    files = [("apps.yaml", manifest, 0o644)]
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
    if len(files) > MAX_ENTRIES:
        raise ValueError("package exceeds Silicon Apps' 20,000 entry limit")
    if sum(len(data) for _, data, _ in files) > MAX_EXPANDED_SIZE:
        raise ValueError("expanded package exceeds Silicon Apps' 1 GiB limit")
    write_archive(output, files)
    if output.stat().st_size > MAX_COMPRESSED_SIZE:
        output.unlink()
        raise ValueError("compressed package exceeds Silicon Apps' 512 MiB limit")
    if standalone_output:
        payloads = {name: (data, mode) for name, data, mode in files}
        for target, triple in STANDALONE_TARGETS.items():
            stem = f"browser-v{version()}-{triple}"
            archive = standalone_output / f"{stem}.tar.gz"
            write_archive(archive, [
                (f"{stem}/browser", *payloads[f"targets/{target}/browser"]),
                (f"{stem}/LICENSE", *payloads[f"targets/{target}/LICENSE-silicon-browser"]),
            ])
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            archive.with_name(archive.name + ".sha256").write_text(f"{digest}  {archive.name}\n")
    print(output)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--targets", type=Path, default=ROOT / "target/apps/targets")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--standalone-output", type=Path)
    parser.add_argument("--stage", choices=TARGETS)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--controller", type=Path)
    args = parser.parse_args()
    if args.stage:
        if not args.binary:
            parser.error("--stage requires --binary")
        stage(args.stage, args.binary, args.controller, args.targets / args.stage)
    else:
        pack(args.targets, args.output or ROOT / f"target/apps-browser-{version()}.tar.gz", args.standalone_output)
