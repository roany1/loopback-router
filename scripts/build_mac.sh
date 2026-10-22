#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano
# Build "Loopback Router.app" on macOS. Requires: Python 3.9+ (python.org build
# recommended: includes Tk), Rust (https://rustup.rs).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -m venv .venv && source .venv/bin/activate
pip install -U pip maturin pyinstaller pytest
cargo test --no-default-features
maturin build --release --out dist/wheels
pip install --force-reinstall dist/wheels/*.whl
pytest -q tests
cd packaging && pyinstaller --noconfirm --distpath ../dist --workpath ../build loopback.spec && cd ..
ditto -c -k --keepParent "dist/Loopback Router.app" "dist/LoopbackRouter-macos-$(uname -m).zip"
echo "Fatto: dist/Loopback Router.app"
