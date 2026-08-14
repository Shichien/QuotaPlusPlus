$ErrorActionPreference = 'Stop'
$files = @('scripts\install.ps1', 'scripts\uninstall.ps1')
$results = foreach ($file in $files) {
    $tokens = $null
    $errors = $null
    [Management.Automation.Language.Parser]::ParseFile(
        (Resolve-Path $file),
        [ref]$tokens,
        [ref]$errors
    ) | Out-Null
    [pscustomobject]@{
        Shell = 'Windows PowerShell 5.1'
        File = $file
        Errors = @($errors).Count
    }
}
$results | Format-Table -AutoSize
if (($results | Measure-Object -Property Errors -Sum).Sum -ne 0) {
    exit 1
}
