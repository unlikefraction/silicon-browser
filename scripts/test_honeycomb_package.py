#!/usr/bin/env python3
"""Offline package contracts; fixtures are headers, never executed as programs."""
import hashlib
import json
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package_honeycomb as package


def executable(target):
    data = bytearray(128)
    arm = target.endswith("aarch64")
    if target.startswith("linux-"):
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", data, 18, 183 if arm else 62)
    elif target.startswith("macos-"):
        data[:4] = b"\xcf\xfa\xed\xfe"
        struct.pack_into("<I", data, 4, 0x100000C if arm else 0x1000007)
    else:
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 60, 64)
        data[64:68] = b"PE\0\0"
        struct.pack_into("<H", data, 68, 0xAA64 if arm else 0x8664)
    return bytes(data)


class PackageTests(unittest.TestCase):
    def test_complete_archive_and_invalid_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            version = package.version()
            manifest = (package.ROOT / "honeycomb.yaml").read_bytes()
            (root / "honeycomb.yaml").write_bytes(manifest)
            (root / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{version}"\n')
            output = root / "package.tar.gz"
            for target in package.TARGETS:
                directory = root / target
                directory.mkdir()
                data = executable(target)
                names = package.binaries(target)
                for name in names:
                    (directory / name).write_bytes(data)
                for name in ["LICENSE-silicon-browser", *package.LICENSES]:
                    (directory / name).write_text("fixture license")
                (directory / ".env").write_text("PRIVATE_TOKEN=secret")
                (directory / "state.json").write_text("secret")
                receipt = {"version": version, "controller_version": package.CONTROLLER_VERSION,
                           "sha256": {name: hashlib.sha256(data).hexdigest() for name in names}}
                (directory / "build.json").write_text(json.dumps(receipt))
            with patch.object(package, "ROOT", root):
                standalone = root / "standalone"
                package.pack(root, output, standalone)
                with tarfile.open(output) as archive:
                    self.assertEqual(len(archive.getnames()), 1 + 6 * 6)
                    self.assertEqual(archive.getnames()[0], "honeycomb.yaml")
                    for entry in archive:
                        self.assertTrue(entry.isfile())
                        self.assertNotIn(entry.name.split("/")[-1], [".env", "state.json", "build.json"])
                        if entry.name.endswith(("/browser", "/browser.exe", "/sb-browser-engine", "/sb-browser-engine.exe")):
                            self.assertEqual(entry.mode, 0o755)
                self.assertEqual(len(list(standalone.glob("*.tar.gz"))), 4)
                for target, triple in package.STANDALONE_TARGETS.items():
                    stem = f"browser-v{version}-{triple}"
                    path = standalone / f"{stem}.tar.gz"
                    self.assertEqual(path.with_name(path.name + ".sha256").read_text(),
                                     f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
                    with tarfile.open(path) as archive:
                        self.assertEqual(archive.getnames(), [f"{stem}/browser", f"{stem}/LICENSE"])
                        self.assertEqual(archive.extractfile(f"{stem}/browser").read(), executable(target))
                        self.assertEqual(archive.getmember(f"{stem}/browser").mode, 0o755)
                directory = root / "windows-aarch64"
                path = directory / "browser.exe"
                original = path.read_bytes()
                for invalid in [b"#!/bin/sh\nexit 0\n", executable("windows-x86_64")]:
                    path.write_bytes(invalid)
                    with self.assertRaisesRegex(ValueError, "architecture"):
                        package.pack(root, output)
                path.write_bytes(original + b"changed")
                with self.assertRaisesRegex(ValueError, "receipt"):
                    package.pack(root, output)
                path.write_bytes(original)
                receipt_path = directory / "build.json"
                receipt = json.loads(receipt_path.read_text())
                receipt["version"] = "0.0.0"
                receipt_path.write_text(json.dumps(receipt))
                with self.assertRaisesRegex(ValueError, "version"):
                    package.pack(root, output)
                (root / "honeycomb.yaml").write_bytes(manifest.replace(b"app_id: browser", b"app_id: tos>browser"))
                with self.assertRaisesRegex(ValueError, "app_id"):
                    package.pack(root, output)
                (root / "honeycomb.yaml").write_bytes(manifest)
                directory.rename(root / "missing-target")
                with self.assertRaisesRegex(ValueError, "target directory"):
                    package.pack(root, output)
                (root / "honeycomb.yaml").write_bytes(manifest.replace(version.encode(), b"0.0.0"))
                with self.assertRaisesRegex(ValueError, "version"):
                    package.pack(root, output)


if __name__ == "__main__":
    unittest.main()
