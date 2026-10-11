param([switch]$Test, [switch]$Package)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    $cargoCommand = 'cargo'
    if (Test-Path -LiteralPath "$env:USERPROFILE\.cargo\bin\cargo.exe") {
        $cargoCommand = "$env:USERPROFILE\.cargo\bin\cargo.exe"
    }
    if ($Test) {
        & $cargoCommand test --locked
        if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
        & $cargoCommand clippy --locked --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
    }
    & $cargoCommand build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Build failed' }
    if ($Package) {
        New-Item -ItemType Directory -Path dist -Force | Out-Null
        $stage = Join-Path $root 'dist\aw-calendar-sync'
        New-Item -ItemType Directory -Path $stage -Force | Out-Null
        Copy-Item -LiteralPath target\release\aw-calendar-sync.exe,README.md,config.example.json,LICENSE -Destination $stage -Force
        Copy-Item -LiteralPath scripts\install.ps1 -Destination $stage -Force
        New-Item -ItemType Directory -Path (Join-Path $stage 'scripts') -Force | Out-Null
        Copy-Item -LiteralPath scripts\migrate-config.py -Destination (Join-Path $stage 'scripts') -Force
        Copy-Item -LiteralPath docs -Destination $stage -Recurse -Force
        Compress-Archive -LiteralPath $stage -DestinationPath dist\aw-calendar-sync-0.1.0-windows-x64.zip -Force
    }
} finally { Pop-Location }
