# Builds the issuerelay CLI in release mode and stages it where Tauri's externalBin looks for it
# (app/src-tauri/tauri.bundle.conf.json): app/src-tauri/binaries/issuerelay-<triple>.exe. Tauri
# strips the triple when it bundles the file next to issuerelay-app.exe. Windows releases are
# x64 only.
#
# Usage: pwsh scripts/stage-cli.ps1 [-Target <triple>]
# Exit codes: 0 = staged; a failed build or copy stops the script with an error.
param([string]$Target = 'x86_64-pc-windows-msvc')
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
Set-Location (Join-Path $PSScriptRoot '..')

cargo build -p issuerelay-cli --release --locked --target $Target
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
$out = 'app/src-tauri/binaries'
New-Item -ItemType Directory -Force -Path $out | Out-Null
$dest = Join-Path $out "issuerelay-$Target.exe"
Copy-Item "target/$Target/release/issuerelay.exe" $dest -Force
Write-Output "staged $dest"
