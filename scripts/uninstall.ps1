$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$installDirectory = if ($env:QPP_INSTALL_DIR) {
    [IO.Path]::GetFullPath($env:QPP_INSTALL_DIR)
} else {
    Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'QuotaPlusPlus\bin'
}
$executable = Join-Path $installDirectory 'qpp.exe'

foreach ($process in @(Get-Process -Name 'qpp' -ErrorAction SilentlyContinue)) {
    try {
        if ($process.Path -and [IO.Path]::GetFullPath($process.Path) -eq $executable) {
            Stop-Process -Id $process.Id -Force
            $process.WaitForExit()
        }
    } catch {
        throw 'Close the running QuotaPlusPlus window and run the uninstaller again.'
    }
}

if (Test-Path -LiteralPath $executable -PathType Leaf) {
    Remove-Item -LiteralPath $executable -Force
}
if ((Test-Path -LiteralPath $installDirectory -PathType Container) -and
    @(Get-ChildItem -LiteralPath $installDirectory -Force).Count -eq 0) {
    Remove-Item -LiteralPath $installDirectory -Force
}

if ($env:QPP_SKIP_PATH -ne '1') {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $remaining = @($userPath -split ';' | Where-Object {
        $_.Trim() -and $_.TrimEnd('\') -ine $installDirectory.TrimEnd('\')
    })
    [Environment]::SetEnvironmentVariable('Path', ($remaining -join ';'), 'User')
}

Write-Host 'QuotaPlusPlus uninstalled.' -ForegroundColor Green
Write-Host 'Codex login, configuration, sessions, and backups were kept.'
