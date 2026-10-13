<#
.SYNOPSIS
Per-user XDM installer for Windows. No administrator rights required.

.DESCRIPTION
Copies the XDM binaries to %LOCALAPPDATA%\Programs\XDM, creates a Start-menu
shortcut, optionally registers xdm-daemon to start with Windows (HKCU Run),
and registers the native messaging host with Chromium-family browsers and
Firefox by calling the existing registration scripts next to this one.
Every created path, shortcut and registry value is recorded in a manifest so
Uninstall-XDM.ps1 removes exactly what this script created. Downloaded files
and the daemon data directory are never removed unless -WhatIf / -Purge say so.

Supports -WhatIf for a full dry run.

.EXAMPLE
.\Install-XDM.ps1 -SourceDir ..\..\target\release -Autostart

.EXAMPLE
.\Install-XDM.ps1 -WhatIf
#>
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    # Directory holding the built .exe files. Defaults to bin\ next to the
    # repository root (tarball layout) or target\release in a checkout.
    [string]$SourceDir,
    # Install root. Defaults to %LOCALAPPDATA%\Programs\XDM.
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\XDM'),
    # Register xdm-daemon to launch at logon via HKCU Run.
    [switch]$Autostart,
    # Chromium-family browsers to register the native host with.
    [ValidateSet('Chrome', 'Chromium', 'Edge')]
    [string[]]$Browser = @('Chrome', 'Edge'),
    # Chromium extension IDs (32 chars a-p). Derived from the unpacked
    # extensions\chromium directory when omitted.
    [string[]]$ExtensionId,
    # Firefox add-on IDs.
    [string[]]$FirefoxExtensionId = @('xdm@xdm.local'),
    [switch]$NoBrowserRegistration
)
$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = (Resolve-Path (Join-Path $scriptDir '..\..')).Path
$manifestDir = Join-Path $env:LOCALAPPDATA 'XDM'
$manifestPath = Join-Path $manifestDir 'install-manifest.json'

function Get-UnpackedExtensionId {
    param([string]$ExtensionDirectory)
    # Chromium derives unpacked-extension IDs from the absolute path of the
    # extension directory: sha256(path), first 16 hex digits mapped to a-p.
    $abs = (Resolve-Path -LiteralPath $ExtensionDirectory).Path.Replace('\', '/')
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($abs))
    }
    finally {
        $sha.Dispose()
    }
    $hex = -join ($bytes[0..7] | ForEach-Object { $_.ToString('x2') })
    $mapped = -join ($hex.ToCharArray() | ForEach-Object {
            $n = if ($_ -ge '0' -and $_ -le '9') { [int]$_ - 48 } else { [int]$_ - 87 }
            [char]($n + [int][char]'a')
        })
    return $mapped
}

if (-not $SourceDir) {
    foreach ($candidate in @((Join-Path $repoRoot 'bin'), (Join-Path $repoRoot 'target\release'))) {
        if (Test-Path -LiteralPath (Join-Path $candidate 'xdm-daemon.exe')) { $SourceDir = $candidate; break }
    }
}
if (-not $SourceDir) { throw 'No built binaries found; pass -SourceDir (expected target\release or bin\)' }
$SourceDir = (Resolve-Path -LiteralPath $SourceDir).Path
foreach ($name in @('xdm-daemon.exe', 'xdm-native-host.exe')) {
    if (-not (Test-Path -LiteralPath (Join-Path $SourceDir $name) -PathType Leaf)) {
        throw "$name missing in $SourceDir"
    }
}

$createdFiles = New-Object System.Collections.Generic.List[string]
$createdDirs = New-Object System.Collections.Generic.List[string]
$createdLinks = New-Object System.Collections.Generic.List[string]
$createdRegistry = New-Object System.Collections.Generic.List[string]

if (-not (Test-Path -LiteralPath $InstallDir)) {
    if ($PSCmdlet.ShouldProcess($InstallDir, 'create install directory')) {
        New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
        $createdDirs.Add($InstallDir)
    }
}

foreach ($name in @('xdm-cli.exe', 'xdm-daemon.exe', 'xdm-gui.exe', 'xdm-native-host.exe')) {
    $src = Join-Path $SourceDir $name
    if (-not (Test-Path -LiteralPath $src -PathType Leaf)) {
        Write-Warning "skipping missing binary: $name"
        continue
    }
    $dst = Join-Path $InstallDir $name
    if ($PSCmdlet.ShouldProcess($dst, 'install binary')) {
        Copy-Item -LiteralPath $src -Destination $dst -Force
        $createdFiles.Add($dst)
    }
}

$gui = Join-Path $InstallDir 'xdm-gui.exe'
if (Test-Path -LiteralPath $gui -PathType Leaf) {
    $startMenu = [Environment]::GetFolderPath('Programs')
    $lnkPath = Join-Path $startMenu 'XDM.lnk'
    if ($PSCmdlet.ShouldProcess($lnkPath, 'create Start-menu shortcut')) {
        $shell = New-Object -ComObject WScript.Shell
        $shortcut = $shell.CreateShortcut($lnkPath)
        $shortcut.TargetPath = $gui
        $shortcut.WorkingDirectory = $InstallDir
        $shortcut.Description = 'XDM download manager'
        $shortcut.Save()
        $createdLinks.Add($lnkPath)
    }
}

$daemon = Join-Path $InstallDir 'xdm-daemon.exe'
if ($Autostart) {
    $runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
    if ($PSCmdlet.ShouldProcess("$runKey\XDM", 'register logon autostart')) {
        if (-not (Test-Path -LiteralPath $runKey)) {
            New-Item -Force -Path $runKey | Out-Null
        }
        Set-ItemProperty -Path $runKey -Name 'XDM' -Value "`"$daemon`""
        $createdRegistry.Add("$runKey\XDM")
    }
}

$hostBinary = Join-Path $InstallDir 'xdm-native-host.exe'
if (-not $NoBrowserRegistration) {
    if (-not $ExtensionId -or $ExtensionId.Count -eq 0) {
        $unpacked = Join-Path $repoRoot 'extensions\chromium'
        if (Test-Path -LiteralPath $unpacked) {
            $ExtensionId = @(Get-UnpackedExtensionId -ExtensionDirectory $unpacked)
            Write-Verbose "derived Chromium extension ID: $ExtensionId"
        }
    }
    if ($ExtensionId -and $ExtensionId.Count -gt 0) {
        if ($PSCmdlet.ShouldProcess('Chromium-family native messaging host', 'register')) {
            & (Join-Path $scriptDir 'Register-NativeHost.ps1') -HostPath $hostBinary -ExtensionId $ExtensionId -Browser $Browser
            $createdFiles.Add((Join-Path $manifestDir 'org.xdm.browser.json'))
            foreach ($b in $Browser) {
                $root = switch ($b) {
                    'Chrome' { 'HKCU:\Software\Google\Chrome\NativeMessagingHosts' }
                    'Chromium' { 'HKCU:\Software\Chromium\NativeMessagingHosts' }
                    'Edge' { 'HKCU:\Software\Microsoft\Edge\NativeMessagingHosts' }
                }
                $createdRegistry.Add("$root\org.xdm.browser")
            }
        }
    }
    else {
        Write-Warning 'no Chromium extension ID available; skipping Chromium-family registration'
    }
    if ($PSCmdlet.ShouldProcess('Firefox native messaging host', 'register')) {
        & (Join-Path $scriptDir 'Register-FirefoxNativeHost.ps1') -HostPath $hostBinary -ExtensionId $FirefoxExtensionId
        $createdFiles.Add((Join-Path $manifestDir 'org.xdm.browser.firefox.json'))
        $createdRegistry.Add('HKCU:\Software\Mozilla\NativeMessagingHosts\org.xdm.browser')
    }
}

if ($PSCmdlet.ShouldProcess($manifestPath, 'write install manifest')) {
    $manifest = [ordered]@{
        version = 1
        installDir = $InstallDir
        files = @($createdFiles)
        dirs = @($createdDirs)
        shortcuts = @($createdLinks)
        registry = @($createdRegistry)
    }
    if (-not (Test-Path -LiteralPath $manifestDir)) {
        New-Item -ItemType Directory -Force -Path $manifestDir | Out-Null
    }
    $manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $manifestPath -Encoding UTF8
    Write-Output "manifest written: $manifestPath"
}

Write-Output 'install complete'
