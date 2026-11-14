param(
    [switch]$SkipBuild,
    [switch]$InstallInno
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $root

$versionLine = Select-String -Path 'Cargo.toml' -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $versionLine) {
    throw 'Could not read the package version from Cargo.toml'
}
$version = $versionLine.Matches[0].Groups[1].Value

function Find-Iscc {
    $candidates = @(
        (Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue).Source,
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 7\ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 7\ISCC.exe"
    )
    $candidates | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
}

$iscc = Find-Iscc

if (-not $iscc -and $InstallInno) {
    Write-Host 'Inno Setup not found, installing with winget...'
    winget install --id JRSoftware.InnoSetup -e --silent `
        --accept-package-agreements --accept-source-agreements
    $iscc = Find-Iscc
}

if (-not $iscc) {
    throw @'
Inno Setup 6 was not found.
Install it with one of these commands, then run this script again:

    winget install -e --id JRSoftware.InnoSetup

or run:

    powershell -ExecutionPolicy Bypass -File build-installer.ps1 -InstallInno
'@
}

if (-not $SkipBuild) {
    Write-Host 'Building release executable...'
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        throw 'cargo build --release failed'
    }
}

if (-not (Test-Path 'target\release\fpscraft.exe')) {
    throw 'target\release\fpscraft.exe is missing; run without -SkipBuild first'
}

New-Item -ItemType Directory -Force 'dist' | Out-Null

Write-Host "Compiling setup with $iscc"
& $iscc "/DAppVersion=$version" 'installer\fpscraft.iss'
if ($LASTEXITCODE -ne 0) {
    throw 'Inno Setup compilation failed'
}

$versioned = "dist\FPScraft-Setup-$version.exe"
$singleFile = 'dist\FPScraft-Setup.exe'
Copy-Item $versioned $singleFile -Force

$hash = (Get-FileHash $singleFile -Algorithm SHA256).Hash
$sizeMb = [math]::Round((Get-Item $singleFile).Length / 1MB, 2)

Write-Host ''
Write-Host "Setup ready: $versioned"
Write-Host "Send this one file: $singleFile ($sizeMb MB)"
Write-Host "SHA256: $hash"
