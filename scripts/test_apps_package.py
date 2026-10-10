#!/usr/bin/env python3
"""Offline package contracts; fixtures are headers, never executed as programs."""
import hashlib
import json
import os
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package_apps as package


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
    def test_apps_requires_strict_release_versions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with patch.object(package, "ROOT", root):
                for value in ["0.3.1-dev", "0.3.1+build", "03.1.0", "0.3"]:
                    (root / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{value}"\n')
                    with self.assertRaisesRegex(ValueError, "strict x.y.z"):
                        package.version()

    def test_complete_archive_and_invalid_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            version = package.version()
            manifest = (package.ROOT / "apps.yaml").read_bytes()
            (root / "apps.yaml").write_bytes(manifest)
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
                repeated = root / "repeated.tar.gz"
                package.pack(root, repeated)
                self.assertEqual(output.read_bytes(), repeated.read_bytes())
                with tarfile.open(output) as archive:
                    self.assertEqual(len(archive.getnames()), 1 + 6 * 6)
                    self.assertEqual(archive.getnames()[0], "apps.yaml")
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
                receipt["version"] = version
                receipt_path.write_text(json.dumps(receipt))
                (root / "apps.yaml").write_bytes(manifest.replace(b"app_id: browser", b"app_id: tos>browser"))
                with self.assertRaisesRegex(ValueError, "app_id"):
                    package.pack(root, output)
                (root / "apps.yaml").write_bytes(manifest)
                for invalid_manifest in [
                    manifest.replace(b"schema_version: 1", b"format_version: 1"),
                    manifest.replace(b"command: browser", b"command: sb"),
                    manifest.replace(b"binary: targets/linux-x86_64/browser", b"binary: ../browser"),
                ]:
                    (root / "apps.yaml").write_bytes(invalid_manifest)
                    with self.assertRaisesRegex(ValueError, "schema_version"):
                        package.pack(root, output)
                (root / "apps.yaml").write_bytes(manifest)
                for constant, message in [
                    ("MAX_EXPANDED_SIZE", "1 GiB"),
                    ("MAX_COMPRESSED_SIZE", "512 MiB"),
                    ("MAX_ENTRIES", "20,000"),
                ]:
                    with patch.object(package, constant, 1):
                        with self.assertRaisesRegex(ValueError, message):
                            package.pack(root, output)
                directory.rename(root / "missing-target")
                with self.assertRaisesRegex(ValueError, "target directory"):
                    package.pack(root, output)
                (root / "apps.yaml").write_bytes(manifest.replace(version.encode(), b"0.0.0"))
                with self.assertRaisesRegex(ValueError, "version"):
                    package.pack(root, output)

    def test_stage_requires_valid_signed_out_discovery_in_isolated_home(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "0.3.1"\n')
            (root / "LICENSE").write_text("fixture license")
            binary = root / "browser"
            controller = root / "controller"
            for path in [binary, controller]:
                path.write_bytes(executable("linux-x86_64"))
            authenticated = False
            caller_home = root / "existing-home"
            caller_home.mkdir()
            (caller_home / "state.json").write_text("private signed-in state")
            homes = set()
            commands = set()

            def check_environment(arguments, **kwargs):
                env = kwargs["env"]
                self.assertEqual({key for key in env if key.startswith("SB_")}, {"SB_HOME", "SB_BACKEND_URL"})
                self.assertEqual(env["SB_BACKEND_URL"], "http://127.0.0.1:1")
                self.assertEqual(env["PATH"], os.environ["PATH"])
                home = Path(env["SB_HOME"])
                self.assertNotEqual(home, caller_home)
                self.assertTrue(home.is_dir())
                self.assertFalse((home / "state.json").exists())
                if os.name != "nt":
                    self.assertEqual(home.stat().st_mode & 0o777, 0o700)
                homes.add(home)
                commands.add((Path(arguments[0]).name, *arguments[1:]))

            def command_output(arguments, **kwargs):
                check_environment(arguments, **kwargs)
                if arguments[1:] == ["--version"]:
                    return "browser 0.3.1" if Path(arguments[0]).name == "browser" else f"agent-browser {package.CONTROLLER_VERSION}"
                if arguments[1:] == ["accounts", "--json"]:
                    return '{"app_id":"browser"}'
                return json.dumps({"authenticated": authenticated})

            with patch.object(package, "ROOT", root), patch.object(package, "download", return_value=b"license"), \
                    patch.object(package.subprocess, "check_output", side_effect=command_output), \
                    patch.object(package.subprocess, "run", side_effect=check_environment), \
                    patch.dict(os.environ, {"SB_HOME": str(caller_home), "SB_AUTHTOKEN": "live-secret",
                                            "SB_ORG_ID": "live-org", "SB_BACKEND_URL": "https://live.example",
                                            "SB_OTHER": "live-setting"}):
                package.stage("linux-x86_64", binary, controller, root / "stage")
                self.assertTrue((root / "stage/build.json").is_file())
                self.assertEqual(len(homes), 1)
                self.assertTrue(all(not home.exists() for home in homes))
                self.assertEqual(commands, {
                    ("browser", "--version"), ("browser", "--help"),
                    ("browser", "accounts", "--json"), ("browser", "login", "status", "--json"),
                    ("sb-browser-engine", "--version"), ("sb-browser-engine", "--help"),
                })
                for invalid in [True, "false", 0, None]:
                    authenticated = invalid
                    with self.assertRaisesRegex(ValueError, "discovery command"):
                        package.stage("linux-x86_64", binary, controller, root / "invalid-stage")
                self.assertTrue(all(not home.exists() for home in homes))
                self.assertEqual((caller_home / "state.json").read_text(), "private signed-in state")


if __name__ == "__main__":
    unittest.main()
