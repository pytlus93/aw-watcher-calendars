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
    Copy-Item -LiteralPath aw-watcher-calendars.exe.actions.json -Destination target\release\aw-watcher-calendars.exe.actions.json -Force
    if ($Package) {
        $manifest = [IO.File]::ReadAllText((Join-Path $root 'Cargo.toml'))
        $version = [regex]::Match($manifest, '(?m)^version\s*=\s*"([0-9.]+)"').Groups[1].Value
        if (-not $version) { throw 'Cannot determine package version' }
        New-Item -ItemType Directory -Path dist -Force | Out-Null
        $stage = Join-Path $root 'dist\aw-watcher-calendars'
        $resolvedStage = [IO.Path]::GetFullPath($stage)
        $distRoot = [IO.Path]::GetFullPath((Join-Path $root 'dist')) + [IO.Path]::DirectorySeparatorChar
        if (-not $resolvedStage.StartsWith($distRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unsafe package staging path' }
        if (Test-Path -LiteralPath $resolvedStage) { Remove-Item -LiteralPath $resolvedStage -Recurse -Force }
        New-Item -ItemType Directory -Path $stage -Force | Out-Null
        Copy-Item -LiteralPath target\release\aw-watcher-calendars.exe,aw-watcher-calendars.exe.actions.json,README.md,CHANGELOG.md,config.example.json,LICENSE -Destination $stage -Force
        Copy-Item -LiteralPath scripts\install.ps1 -Destination $stage -Force
        New-Item -ItemType Directory -Path (Join-Path $stage 'scripts') -Force | Out-Null
        Copy-Item -LiteralPath scripts\migrate-config.py -Destination (Join-Path $stage 'scripts') -Force
        Copy-Item -LiteralPath docs -Destination $stage -Recurse -Force
        $zip = Join-Path $root "dist\aw-watcher-calendars-$version-windows-x64.zip"
        Compress-Archive -LiteralPath $stage -DestinationPath $zip -Force
        $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText("$zip.sha256", "$hash  $([IO.Path]::GetFileName($zip))`n")
        Write-Output "Release package: $zip"
    }
} finally { Pop-Location }
