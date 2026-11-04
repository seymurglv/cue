# Puts the tools llama.cpp's build needs within reach of cargo: CMake and Ninja from Visual Studio Build Tools, and
# libclang (LLVM) for the Rust bindings. Dot-source it before a build that compiles cue-engines:
#
#   . scripts\dev-env.ps1
#
# Nothing is installed here. LLVM comes from `winget install --id LLVM.LLVM -e`.
$ErrorActionPreference = 'Stop'

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
$vs = $null
if (Test-Path $vswhere) {
    $vs = & $vswhere -latest -products * -property installationPath 2>$null | Select-Object -First 1
}
if (-not $vs) {
    $vs = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\2022\BuildTools'
}
$cmake = Join-Path $vs 'Common7\IDE\CommonExtensions\Microsoft\CMake'
foreach ($dir in @((Join-Path $cmake 'CMake\bin'), (Join-Path $cmake 'Ninja'))) {
    if (-not (Test-Path $dir)) { throw "Not found: $dir (Visual Studio Build Tools with CMake)" }
    if (($env:PATH -split ';') -notcontains $dir) { $env:PATH = "$dir;$env:PATH" }
}

$llvm = Join-Path $env:ProgramFiles 'LLVM\bin'
if (-not (Test-Path (Join-Path $llvm 'libclang.dll'))) {
    throw "Not found: $llvm\libclang.dll (install LLVM: winget install --id LLVM.LLVM -e)"
}
$env:LIBCLANG_PATH = $llvm
