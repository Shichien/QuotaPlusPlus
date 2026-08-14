$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

$repository = if ($env:QPP_REPOSITORY) {
    $env:QPP_REPOSITORY
} else {
    '__QPP_REPOSITORY__'
}

if ($repository.Contains('__QPP_')) {
    throw 'Installer repository is not configured. Use a release-generated install.ps1.'
}

$architecture = if ($env:PROCESSOR_ARCHITEW6432) {
    $env:PROCESSOR_ARCHITEW6432
} else {
    $env:PROCESSOR_ARCHITECTURE
}
if ($architecture -ne 'AMD64') {
    throw "This installer currently provides Windows x64 builds. Detected: $architecture"
}

$installDirectory = if ($env:QPP_INSTALL_DIR) {
    [IO.Path]::GetFullPath($env:QPP_INSTALL_DIR)
} else {
    Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'QuotaPlusPlus\bin'
}
$executable = Join-Path $installDirectory 'qpp.exe'
$download = Join-Path $installDirectory ".qpp-download-$PID.exe"
$source = if ($env:QPP_INSTALL_SOURCE) {
    $env:QPP_INSTALL_SOURCE
} else {
    "https://github.com/$repository/releases/latest/download/qpp-windows-x64.exe"
}

New-Item -ItemType Directory -Path $installDirectory -Force | Out-Null

try {
    if ([Uri]::IsWellFormedUriString($source, [UriKind]::Absolute)) {
        Invoke-WebRequest -UseBasicParsing -Uri $source -OutFile $download
    } else {
        Copy-Item -LiteralPath $source -Destination $download -Force
    }

    $downloadInfo = Get-Item -LiteralPath $download
    if ($downloadInfo.Length -lt 1024KB) {
        throw 'Downloaded file is unexpectedly small.'
    }
    $stream = [IO.File]::OpenRead($download)
    try {
        if ($stream.ReadByte() -ne 0x4D -or $stream.ReadByte() -ne 0x5A) {
            throw 'Downloaded file is not a Windows executable.'
        }
    } finally {
        $stream.Dispose()
    }

    if (Test-Path -LiteralPath $executable -PathType Leaf) {
        foreach ($process in @(Get-Process -Name 'qpp' -ErrorAction SilentlyContinue)) {
            try {
                if ($process.Path -and [IO.Path]::GetFullPath($process.Path) -eq $executable) {
                    Stop-Process -Id $process.Id -Force
                    $process.WaitForExit()
                }
            } catch {
                throw 'Close the running QuotaPlusPlus window and run the installer again.'
            }
        }
    }

    Move-Item -LiteralPath $download -Destination $executable -Force
} finally {
    if (Test-Path -LiteralPath $download) {
        Remove-Item -LiteralPath $download -Force
    }
}

if ($env:QPP_SKIP_PATH -ne '1') {
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $pathEntries = @($userPath -split ';' | Where-Object { $_.Trim() })
    $alreadyPresent = $pathEntries | Where-Object {
        $_.TrimEnd('\') -ieq $installDirectory.TrimEnd('\')
    }
    if (-not $alreadyPresent) {
        $updatedPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
            $installDirectory
        } else {
            "$($userPath.TrimEnd(';'));$installDirectory"
        }
        [Environment]::SetEnvironmentVariable('Path', $updatedPath, 'User')
    }

    $currentEntries = @($env:Path -split ';')
    if (-not ($currentEntries | Where-Object { $_.TrimEnd('\') -ieq $installDirectory.TrimEnd('\') })) {
        $env:Path = "$($env:Path.TrimEnd(';'));$installDirectory"
    }
}

Write-Host "QuotaPlusPlus installed: $executable" -ForegroundColor Green
Write-Host 'Command: qpp'

if ($env:QPP_NO_LAUNCH -ne '1') {
    Start-Process -FilePath $executable
}
