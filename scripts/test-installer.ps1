$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$projectRoot = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path $projectRoot '.installer-test'
$installDirectory = Join-Path $fixtureRoot 'bin'
$source = Join-Path $projectRoot 'dist\qpp.exe'

if (Test-Path -LiteralPath $fixtureRoot) {
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null

$previous = @{
    QPP_REPOSITORY = $env:QPP_REPOSITORY
    QPP_INSTALL_DIR = $env:QPP_INSTALL_DIR
    QPP_INSTALL_SOURCE = $env:QPP_INSTALL_SOURCE
    QPP_SKIP_PATH = $env:QPP_SKIP_PATH
    QPP_NO_LAUNCH = $env:QPP_NO_LAUNCH
}

try {
    $env:QPP_REPOSITORY = 'fixture/QuotaPlusPlus'
    $env:QPP_INSTALL_DIR = $installDirectory
    $env:QPP_INSTALL_SOURCE = $source
    $env:QPP_SKIP_PATH = '1'
    $env:QPP_NO_LAUNCH = '1'

    & (Join-Path $PSScriptRoot 'install.ps1')
    $installed = Join-Path $installDirectory 'qpp.exe'
    if (-not (Test-Path -LiteralPath $installed -PathType Leaf)) {
        throw 'Installer did not create qpp.exe.'
    }
    $firstLength = (Get-Item -LiteralPath $installed).Length
    if ($firstLength -ne (Get-Item -LiteralPath $source).Length) {
        throw 'Installed executable does not match the release asset size.'
    }

    & (Join-Path $PSScriptRoot 'install.ps1')
    if ((Get-Item -LiteralPath $installed).Length -ne $firstLength) {
        throw 'Upgrade changed the executable unexpectedly.'
    }

    & (Join-Path $PSScriptRoot 'uninstall.ps1')
    if (Test-Path -LiteralPath $installed) {
        throw 'Uninstaller left qpp.exe behind.'
    }

    Write-Host 'Installer smoke test passed.' -ForegroundColor Green
} finally {
    foreach ($name in $previous.Keys) {
        Set-Item -LiteralPath "Env:$name" -Value $previous[$name]
    }
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
