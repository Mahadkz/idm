<#
.SYNOPSIS
Remove a per-user XDM installation using the manifest written by Install-XDM.ps1.

.DESCRIPTION
Removes exactly the files, Start-menu shortcut, and HKCU registry values the
install manifest records, then the manifest itself. Downloaded files and the
daemon data directory (%LOCALAPPDATA%\XDM) survive unless -Purge is given.
Supports -WhatIf for a full dry run.

.EXAMPLE
.\Uninstall-XDM.ps1

.EXAMPLE
.\Uninstall-XDM.ps1 -Purge -WhatIf
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$ManifestPath = (Join-Path $env:LOCALAPPDATA 'XDM\install-manifest.json'),
    # Also delete the daemon data directory (task database, tokens).
    [switch]$Purge
)
$ErrorActionPreference = 'Stop'

$dataDir = Join-Path $env:LOCALAPPDATA 'XDM'

if (-not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
    throw "no install manifest at $ManifestPath; nothing to uninstall"
}
$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json

foreach ($key in @($manifest.registry)) {
    if (-not $key) { continue }
    if ($PSCmdlet.ShouldProcess($key, 'remove registry key')) {
        # Remove only the recorded leaf key (e.g. ...\NativeMessagingHosts\
        # org.xdm.browser); sibling native hosts belong to other applications.
        Remove-Item -LiteralPath $key -Recurse -Force -ErrorAction SilentlyContinue
    }
}

foreach ($link in @($manifest.shortcuts)) {
    if (-not $link) { continue }
    if ($PSCmdlet.ShouldProcess($link, 'remove Start-menu shortcut')) {
        Remove-Item -LiteralPath $link -Force -ErrorAction SilentlyContinue
    }
}

foreach ($file in @($manifest.files)) {
    if (-not $file) { continue }
    if ($PSCmdlet.ShouldProcess($file, 'remove file')) {
        Remove-Item -LiteralPath $file -Force -ErrorAction SilentlyContinue
    }
}

# Directories are only removed when empty, so user data placed alongside the
# installation is preserved.
foreach ($dir in @($manifest.files | ForEach-Object { Split-Path -Parent $_ } | Sort-Object -Unique)) {
    if (-not $dir -or -not (Test-Path -LiteralPath $dir)) { continue }
    $items = @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue)
    if ($items.Count -eq 0 -and $PSCmdlet.ShouldProcess($dir, 'remove empty directory')) {
        Remove-Item -LiteralPath $dir -Force -ErrorAction SilentlyContinue
    }
}
foreach ($dir in @($manifest.dirs)) {
    if (-not $dir -or -not (Test-Path -LiteralPath $dir)) { continue }
    $items = @(Get-ChildItem -LiteralPath $dir -Force -ErrorAction SilentlyContinue)
    if ($items.Count -eq 0 -and $PSCmdlet.ShouldProcess($dir, 'remove empty directory')) {
        Remove-Item -LiteralPath $dir -Force -ErrorAction SilentlyContinue
    }
}

if ($PSCmdlet.ShouldProcess($ManifestPath, 'remove install manifest')) {
    Remove-Item -LiteralPath $ManifestPath -Force -ErrorAction SilentlyContinue
}

if ($Purge) {
    if (Test-Path -LiteralPath $dataDir) {
        if ($PSCmdlet.ShouldProcess($dataDir, 'purge daemon data directory')) {
            Remove-Item -LiteralPath $dataDir -Recurse -Force
            Write-Output "purged data directory: $dataDir"
        }
    }
}
else {
    if (Test-Path -LiteralPath $dataDir) {
        Write-Output "kept data directory $dataDir (use -Purge to delete it)"
    }
}

Write-Output 'uninstall complete'
