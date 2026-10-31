<#
.SYNOPSIS
Builds the CUE installer: the release program and a per-user NSIS installer, in target\dist.

.DESCRIPTION
Cargo builds into target\dist, never into target\release, where a developer's everyday CUE lives. Paths of this
computer are cut out of the program: rustc writes the user profile as "~" and the repository as "." in every path it
embeds (panic locations, debug strings). The flags go in CARGO_ENCODED_RUSTFLAGS, which cargo reads before RUSTFLAGS
(an empty one left in the shell would win over RUSTFLAGS) and in which a path with a space stays whole. The shell's
CARGO_TARGET_DIR, RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS are put back afterwards, whether the build succeeds or not.

The first installer build downloads NSIS and its Tauri plugin into %LOCALAPPDATA%\tauri, once.

-ScanBuild makes the same installer without compression, to look inside it: `cue-cli leak-scan` reads its files as
they are. The setting reaches the Tauri CLI through a config file written for the build into target\dist, which the
CLI merges over tauri.conf.json; tauri.conf.json is left as it is. That installer is moved to target\dist\scan.

.EXAMPLE
scripts\build-release.ps1

.EXAMPLE
scripts\build-release.ps1 -ScanBuild
#>
[CmdletBinding()]
param([switch]$ScanBuild)

$ErrorActionPreference = 'Stop'

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$dist = Join-Path $repo 'target\dist'
$app = Join-Path $repo 'app'
$profileDir = $env:USERPROFILE.TrimEnd('\')

$saved = @{}
foreach ($name in 'CARGO_TARGET_DIR', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
Push-Location $app
try {
    $env:CARGO_TARGET_DIR = $dist
    # rustc applies the last prefix that matches, so the repository (inside the profile) comes after the profile.
    $remap = @("--remap-path-prefix=$profileDir=~", "--remap-path-prefix=$repo=.")
    $env:CARGO_ENCODED_RUSTFLAGS = $remap -join [char]0x1f

    $tauriArgs = @('run', 'tauri', '--', 'build')
    if ($ScanBuild) {
        New-Item -ItemType Directory -Force -Path $dist | Out-Null
        $patch = Join-Path $dist 'scan-build.json'
        Set-Content -LiteralPath $patch -Encoding ascii -Value '{"bundle":{"windows":{"nsis":{"compression":"none"}}}}'
        $tauriArgs += @('--config', $patch)
    }
    & npm @tauriArgs
    if ($LASTEXITCODE -ne 0) {
        throw "The build failed (exit code $LASTEXITCODE)."
    }

    $maps = @(Get-ChildItem -LiteralPath (Join-Path $app 'dist') -Recurse -Filter '*.map' -File)
    if ($maps.Count -gt 0) {
        throw "app\dist holds source maps, which carry the sources and their paths: $($maps.Count) found."
    }

    $version = (Get-Content -LiteralPath (Join-Path $app 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
    $installer = Join-Path $dist "release\bundle\nsis\CUE_${version}_x64-setup.exe"
    if (-not (Test-Path -LiteralPath $installer)) {
        throw "The installer is not where it should be: $installer"
    }
    if ($ScanBuild) {
        $scanDir = Join-Path $dist 'scan'
        New-Item -ItemType Directory -Force -Path $scanDir | Out-Null
        Move-Item -LiteralPath $installer -Destination $scanDir -Force
        $installer = Join-Path $scanDir (Split-Path -Leaf $installer)
        Write-Host "Uncompressed installer, for the leak scan only: $installer"
    } else {
        Write-Host "Installer: $installer"
    }
} finally {
    Pop-Location
    foreach ($name in $saved.Keys) {
        # A $null passed to .NET becomes "", which would leave the variable set but empty; NullString removes it.
        $value = if ($null -eq $saved[$name]) { [NullString]::Value } else { $saved[$name] }
        [Environment]::SetEnvironmentVariable($name, $value, 'Process')
    }
}
