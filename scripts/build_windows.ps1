# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 Roany Romano
# Build "Loopback Router.exe" on Windows. Requires: Python 3.9+ from python.org
# (with Tcl/Tk), Rust via rustup with the MSVC toolchain (Visual Studio Build Tools).
$ErrorActionPreference = "Stop"
# PowerShell 7.3+: also stop when a native command (cargo, pip, pytest...) fails
$PSNativeCommandUseErrorActionPreference = $true
Set-Location (Join-Path $PSScriptRoot "..")
python -m venv .venv
.\.venv\Scripts\Activate.ps1
python -m pip install -U pip maturin pyinstaller pytest
cargo test --no-default-features
maturin build --release --out dist\wheels
python -m pip install --force-reinstall (Get-ChildItem dist\wheels\*.whl).FullName
python -m pytest -q tests
Push-Location packaging
pyinstaller --noconfirm --distpath ..\dist --workpath ..\build loopback.spec
Pop-Location
Compress-Archive -Force -Path "dist\Loopback Router" -DestinationPath "dist\LoopbackRouter-windows-x64.zip"
Write-Host "Fatto: dist\Loopback Router\Loopback Router.exe"
