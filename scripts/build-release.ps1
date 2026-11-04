<#
.SYNOPSIS
Builds the CUE installer: the release program and a per-user NSIS installer, in target\dist.

.DESCRIPTION
Cargo builds into target\dist, never into target\release, where a developer's everyday CUE lives. Paths of this
computer are cut out of the program: rustc writes the user profile as "~" and the repository as "." in every path it
embeds (panic locations, debug strings). The flags go in CARGO_ENCODED_RUSTFLAGS, which cargo reads before RUSTFLAGS
(an empty one left in the shell would win over RUSTFLAGS) and in which a path with a space stays whole. The shell's
CARGO_TARGET_DIR, RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS are put back afterwards, whether the build succeeds or not. The
C and C++ of llama.cpp, which CMake compiles, get /d1trimfile in CL (read by cl.exe) for the same: put back too.

The first installer build downloads NSIS and its Tauri plugin into %LOCALAPPDATA%\tauri, once.

The installer carries DirectML.dll beside cue.exe: the speech model's runtime (ONNX Runtime, linked into the program)
is built with DirectML and loads it at start, though CUE runs the model on the processor. Cargo copies the DLL into
target\dist\release with the build; a config file written for the build into target\dist adds it to the installer's
resources, and the Tauri CLI merges that file over tauri.conf.json, which is left as it is.

-ScanBuild makes the same installer without compression, to look inside it: `cue-cli leak-scan` reads its files as
they are. The setting reaches the Tauri CLI through the same config file. That installer is moved to target\dist\scan. Both
builds write the installer to the same place, so an installer already there is moved aside to target\dist\aside for
the scan build and put back afterwards, whether the build succeeds or not: both installers stay, in either order. The
program, target\dist\release\cue.exe, is the last build's: build the shipped installer last, and the program the scan
reads is the one it holds.

.EXAMPLE
scripts\build-release.ps1

.EXAMPLE
scripts\build-release.ps1 -ScanBuild
#>
[CmdletBinding()]
param([switch]$ScanBuild)

$ErrorActionPreference = 'Stop'
# CMake, Ninja and libclang for llama.cpp's build.
. (Join-Path $PSScriptRoot 'dev-env.ps1')

$repo = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$dist = Join-Path $repo 'target\dist'
$app = Join-Path $repo 'app'
$profileDir = $env:USERPROFILE.TrimEnd('\')
$version = (Get-Content -LiteralPath (Join-Path $app 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).version
$installer = Join-Path $dist "release\bundle\nsis\CUE_${version}_x64-setup.exe"
$aside = $null

$saved = @{}
foreach ($name in 'CARGO_TARGET_DIR', 'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CL') {
    $saved[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
Push-Location $app
try {
    $env:CARGO_TARGET_DIR = $dist
    # rustc applies the last prefix that matches, so the repository (inside the profile) comes after the profile.
    $remap = @("--remap-path-prefix=$profileDir=~", "--remap-path-prefix=$repo=.")
    $env:CARGO_ENCODED_RUSTFLAGS = $remap -join [char]0x1f
    # llama.cpp's C and C++ (built by CMake, not rustc) write their source paths into asserts: MSVC trims the profile
    # off them, in both spellings CMake may pass. cl.exe reads CL itself; CMake drops CFLAGS on the way.
    $trim = "/d1trimfile:$profileDir\ /d1trimfile:$($profileDir.Replace('\', '/'))/"
    $env:CL = $trim

    $tauriArgs = @('run', 'tauri', '--', 'build')
    New-Item -ItemType Directory -Force -Path $dist | Out-Null
    # Paths in the config are relative to app\src-tauri, like tauri.conf.json's own.
    $bundle = @{ resources = @{ '../../target/dist/release/DirectML.dll' = 'DirectML.dll' } }
    if ($ScanBuild) {
        $bundle.windows = @{ nsis = @{ compression = 'none' } }
    }
    $patch = Join-Path $dist 'build-config.json'
    Set-Content -LiteralPath $patch -Encoding ascii -Value (@{ bundle = $bundle } | ConvertTo-Json -Depth 5 -Compress)
    $tauriArgs += @('--config', $patch)
    if ($ScanBuild) {
        if (Test-Path -LiteralPath $installer) {
            $asideDir = Join-Path $dist 'aside'
            New-Item -ItemType Directory -Force -Path $asideDir | Out-Null
            $kept = Join-Path $asideDir (Split-Path -Leaf $installer)
            Move-Item -LiteralPath $installer -Destination $kept -Force
            $aside = $kept
        }
    }
    & npm @tauriArgs
    if ($LASTEXITCODE -ne 0) {
        throw "The build failed (exit code $LASTEXITCODE)."
    }

    $maps = @(Get-ChildItem -LiteralPath (Join-Path $app 'dist') -Recurse -Filter '*.map' -File)
    if ($maps.Count -gt 0) {
        throw "app\dist holds source maps, which carry the sources and their paths: $($maps.Count) found."
    }

    if (-not (Test-Path -LiteralPath $installer)) {
        throw "The installer is not where it should be: $installer"
    }
    if ($ScanBuild) {
        $scanDir = Join-Path $dist 'scan'
        New-Item -ItemType Directory -Force -Path $scanDir | Out-Null
        Move-Item -LiteralPath $installer -Destination $scanDir -Force
        Write-Host "Uncompressed installer, for the leak scan only: $(Join-Path $scanDir (Split-Path -Leaf $installer))"
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
    if ($aside) {
        # Over whatever a failed scan build left in its place.
        Move-Item -LiteralPath $aside -Destination $installer -Force
        Write-Host "Installer put back: $installer"
    }
}
