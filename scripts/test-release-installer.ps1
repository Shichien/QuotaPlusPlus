$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$projectRoot = Split-Path -Parent $PSScriptRoot
$fixtureRoot = Join-Path $projectRoot '.release-installer-test'
$installDirectory = Join-Path $fixtureRoot 'bin'
$generatedInstaller = Join-Path $fixtureRoot 'install.ps1'

if (Test-Path -LiteralPath $fixtureRoot) {
    Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
}
New-Item -ItemType Directory -Path $fixtureRoot | Out-Null

$previous = @{
    QPP_INSTALL_DIR = $env:QPP_INSTALL_DIR
    QPP_INSTALL_SOURCE = $env:QPP_INSTALL_SOURCE
    QPP_SKIP_PATH = $env:QPP_SKIP_PATH
    QPP_NO_LAUNCH = $env:QPP_NO_LAUNCH
}

try {
    $content = (Get-Content -LiteralPath (Join-Path $PSScriptRoot 'install.ps1') -Raw)
    $content = $content.Replace('__QPP_REPOSITORY__', 'fixture/QuotaPlusPlus')
    Set-Content -LiteralPath $generatedInstaller -Value $content -Encoding utf8NoBOM -NoNewline
    if ((Get-Content -LiteralPath $generatedInstaller -Raw).Contains('__QPP_REPOSITORY__')) {
        throw 'Repository placeholder remains in the release installer.'
    }

    $env:QPP_INSTALL_DIR = $installDirectory
    $env:QPP_INSTALL_SOURCE = Join-Path $projectRoot 'dist\qpp.exe'
    $env:QPP_SKIP_PATH = '1'
    $env:QPP_NO_LAUNCH = '1'

    $scriptText = Get-Content -LiteralPath $generatedInstaller -Raw
    & ([ScriptBlock]::Create($scriptText))
    $installed = Join-Path $installDirectory 'qpp.exe'
    if (-not (Test-Path -LiteralPath $installed -PathType Leaf)) {
        throw 'Release-generated installer did not install qpp.exe.'
    }

    & (Join-Path $PSScriptRoot 'uninstall.ps1')
    if (Test-Path -LiteralPath $installed) {
        throw 'Release installer test left qpp.exe behind.'
    }

    Write-Host 'Release installer pipeline test passed.' -ForegroundColor Green
} finally {
    foreach ($name in $previous.Keys) {
        Set-Item -LiteralPath "Env:$name" -Value $previous[$name]
    }
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
