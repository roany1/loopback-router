# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano
#
# PyInstaller spec: `pyinstaller packaging/loopback.spec` (run from the repo root,
# after installing the wheel built by maturin into the same Python environment).
import sys

APP = "Loopback Router"
a = Analysis(
    ["launcher.py"],
    pathex=[],
    hiddenimports=["loopback._engine"],
    # Apache-2.0 4(a)/(d): license, NOTICE and third-party notices ship with the app.
    datas=[("../LICENSE", "."), ("../NOTICE", "."), ("../THIRD-PARTY-NOTICES.md", ".")],
    excludes=["numpy", "pytest"],
)
pyz = PYZ(a.pure)
exe = EXE(
    pyz, a.scripts, [],
    exclude_binaries=True,
    name=APP,
    console=False,          # GUI app: no terminal window on Windows
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,  # ad-hoc signed by PyInstaller on macOS
    version="version_info.txt" if sys.platform.startswith("win") else None,
)
coll = COLLECT(exe, a.binaries, a.datas, name=APP)

if sys.platform == "darwin":
    app = BUNDLE(
        coll,
        name=f"{APP}.app",
        bundle_identifier="it.roany.loopbackrouter",
        info_plist={
            "CFBundleShortVersionString": "0.1.0",
            "NSHumanReadableCopyright": "Copyright 2026 Roany Romano. Apache License 2.0.",
            "NSHighResolutionCapable": True,
            # Without this key macOS silently feeds zeros to (or kills) apps that
            # open an input device. The text is shown in the permission prompt.
            "NSMicrophoneUsageDescription":
                "Loopback Router legge l'audio dai dispositivi di ingresso per instradarlo verso altre app.",
            "LSMinimumSystemVersion": "11.0",
        },
    )
