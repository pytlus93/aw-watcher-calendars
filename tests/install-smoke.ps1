param([string]$Executable = (Join-Path (Split-Path -Parent $PSScriptRoot) 'target\release\aw-watcher-calendars.exe'))
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$testRoot = Join-Path $projectRoot 'local\installer-smoke'
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$previousLocalAppData = $env:LOCALAPPDATA
$previousProfile = $env:AW_PROFILE
try {
    $env:LOCALAPPDATA = Join-Path $testRoot 'appdata'
    $env:AW_PROFILE = ''
    $mockInstall = Join-Path $testRoot 'activitywatch'
    New-Item -ItemType Directory -Path $mockInstall -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $mockInstall 'aw-qt.exe'),'mock marker; not executable')
    $qtDir = Join-Path $env:LOCALAPPDATA 'activitywatch\activitywatch\aw-qt'
    New-Item -ItemType Directory -Path $qtDir -Force | Out-Null
    $qtConfig = Join-Path $qtDir 'aw-qt.toml'
    $cases = @(
        '',
        '[aw-qt]',
        "[aw-qt]`r`n#autostart_modules = [`"aw-server`"]`r`n[aw-qt-testing]`r`nmarker = true`r`n",
        "[aw-qt]`r`nautostart_modules = [`"aw-server-rust`", `"aw-watcher-vpn`"]`r`n",
        "[aw-qt]`r`nautostart_modules = [`r`n  `"aw-server-rust`", # keep it`r`n  'aw-watcher-vpn',`r`n]`r`n"
    )
    foreach ($text in $cases) {
        [IO.File]::WriteAllText($qtConfig,$text)
        & (Join-Path $projectRoot 'scripts\install.ps1') -ActivityWatchDirectory $mockInstall -Executable $Executable | Out-Null
        python -c "import sys,tomllib; c=tomllib.load(open(sys.argv[1],'rb')); a=c['aw-qt']['autostart_modules']; assert a.count('aw-watcher-calendars')==1; assert ('aw-watcher-vpn' not in sys.argv[2]) or 'aw-watcher-vpn' in a; assert ('aw-qt-testing' not in sys.argv[2]) or c['aw-qt-testing']['marker'] is True" $qtConfig $text
        if ($LASTEXITCODE -ne 0) { throw 'Installer produced invalid TOML or lost modules' }
        $first = [IO.File]::ReadAllText($qtConfig)
        $installedActions = Join-Path $mockInstall 'aw-watcher-calendars\aw-watcher-calendars.exe.actions.json'
        if (-not (Test-Path -LiteralPath $installedActions)) { throw 'Action sidecar missing' }
        if ((Get-Content -LiteralPath $installedActions -Raw | ConvertFrom-Json).module -ne 'aw-watcher-calendars') { throw 'Wrong action sidecar' }
        & (Join-Path $projectRoot 'scripts\install.ps1') -ActivityWatchDirectory $mockInstall -Executable $Executable | Out-Null
        if ([IO.File]::ReadAllText($qtConfig) -ne $first) { throw 'Installer is not idempotent' }
    }
    $config = Get-Content -LiteralPath (Join-Path $env:LOCALAPPDATA 'activitywatch\activitywatch\aw-watcher-calendars\config.json') -Raw | ConvertFrom-Json
    if ($config.poll_seconds -ne 3600 -or $config.lookback_days -ne 7) { throw 'Incorrect default configuration' }
    $legacyExe = Join-Path $testRoot 'legacy.exe'
    Copy-Item -LiteralPath $Executable -Destination $legacyExe -Force
    & (Join-Path $projectRoot 'scripts\install.ps1') -ActivityWatchDirectory $mockInstall -Executable $legacyExe -NoAutostart | Out-Null
    if (Test-Path -LiteralPath $installedActions) { throw 'Rollback left a stale action sidecar' }
    Write-Host 'Installer smoke tests passed (5 configurations, repeated installation).'
} finally { $env:LOCALAPPDATA = $previousLocalAppData; $env:AW_PROFILE = $previousProfile }
