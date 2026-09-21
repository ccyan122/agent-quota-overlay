$ErrorActionPreference = "Stop"

$installDirectory = "C:\Program Files\LLM Quota Overlay"
$source = "D:\llm-usage-ui\src-tauri\target\release\llm-quota-overlay.exe"
$target = Join-Path $installDirectory "llm-quota-overlay.exe"
$backup = Join-Path $installDirectory "llm-quota-overlay.pre-0.2.0.exe"
$statusPath = "D:\llm-usage-ui\release\installed-update-status.json"

try {
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) {
        throw "Release executable is missing."
    }
    if (-not (Test-Path -LiteralPath $target -PathType Leaf)) {
        throw "Installed executable is missing."
    }

    Get-Process -Name "llm-quota-overlay" -ErrorAction SilentlyContinue |
        Where-Object { $_.Path -eq $target } |
        Stop-Process -Force -ErrorAction Stop

    $unlocked = $false
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        try {
            $stream = [System.IO.File]::Open(
                $target,
                [System.IO.FileMode]::Open,
                [System.IO.FileAccess]::ReadWrite,
                [System.IO.FileShare]::None
            )
            $stream.Dispose()
            $unlocked = $true
            break
        } catch {
            Start-Sleep -Milliseconds 100
        }
    }
    if (-not $unlocked) {
        throw "Installed executable remained locked after the overlay stopped."
    }

    if (-not (Test-Path -LiteralPath $backup)) {
        Copy-Item -LiteralPath $target -Destination $backup
    }
    Copy-Item -LiteralPath $source -Destination $target -Force

    $sourceHash = (Get-FileHash -LiteralPath $source -Algorithm SHA256).Hash
    $targetHash = (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash
    if ($sourceHash -ne $targetHash) {
        throw "Installed executable hash does not match the release build."
    }

    Start-Process -FilePath $target -WorkingDirectory $installDirectory -WindowStyle Hidden
    @{
        status = "ok"
        sourceHash = $sourceHash
        targetHash = $targetHash
        backup = $backup
        installedAt = (Get-Date).ToString("o")
    } | ConvertTo-Json | Set-Content -LiteralPath $statusPath -Encoding UTF8
} catch {
    @{
        status = "error"
        message = $_.Exception.Message
        failedAt = (Get-Date).ToString("o")
    } | ConvertTo-Json | Set-Content -LiteralPath $statusPath -Encoding UTF8
    throw
}
