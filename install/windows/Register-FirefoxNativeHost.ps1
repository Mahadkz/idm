param(
    [Parameter(Mandatory=$true)][string]$HostPath,
    [string[]]$ExtensionId = @('xdm@xdm.local')
)
$ErrorActionPreference = 'Stop'
$resolved = (Resolve-Path -LiteralPath $HostPath).Path
if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { throw 'Host executable not found' }
foreach ($id in $ExtensionId) {
    if ($id -cnotmatch '^([A-Za-z0-9._+-]+@[A-Za-z0-9._-]+|\{[0-9a-fA-F-]{36}\})$') { throw "Invalid Firefox add-on ID: $id" }
}
$manifestDir = Join-Path $env:LOCALAPPDATA 'XDM'
New-Item -ItemType Directory -Force -Path $manifestDir | Out-Null
$manifestPath = Join-Path $manifestDir 'org.xdm.browser.firefox.json'
$manifest = @{
    name = 'org.xdm.browser'
    description = 'XDM native messaging bridge'
    path = $resolved
    type = 'stdio'
    allowed_extensions = @($ExtensionId | Sort-Object -Unique)
}
[System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), (New-Object System.Text.UTF8Encoding($false)))
$key = 'HKCU:\Software\Mozilla\NativeMessagingHosts\org.xdm.browser'
New-Item -Force -Path $key | Out-Null
Set-Item -Path $key -Value $manifestPath
Write-Output "Firefox: $key -> $manifestPath"
