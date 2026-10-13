param(
    [Parameter(Mandatory=$true)][string]$HostPath,
    [Parameter(Mandatory=$true)][string[]]$ExtensionId,
    [ValidateSet('Chrome','Chromium','Edge')][string[]]$Browser = @('Chrome','Edge')
)
$ErrorActionPreference = 'Stop'
$resolved = (Resolve-Path -LiteralPath $HostPath).Path
if (-not (Test-Path -LiteralPath $resolved -PathType Leaf)) { throw 'Host executable not found' }
foreach ($id in $ExtensionId) {
    if ($id -cnotmatch '^[a-p]{32}$') { throw "Invalid Chromium extension ID: $id" }
}
$manifestDir = Join-Path $env:LOCALAPPDATA 'XDM'
New-Item -ItemType Directory -Force -Path $manifestDir | Out-Null
$manifestPath = Join-Path $manifestDir 'org.xdm.browser.json'
$manifest = @{
    name = 'org.xdm.browser'
    description = 'XDM native messaging bridge'
    path = $resolved
    type = 'stdio'
    allowed_origins = @($ExtensionId | Sort-Object -Unique | ForEach-Object { "chrome-extension://$_/" })
}
[System.IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), (New-Object System.Text.UTF8Encoding($false)))
$roots = @{
    Chrome = 'HKCU:\Software\Google\Chrome\NativeMessagingHosts'
    Chromium = 'HKCU:\Software\Chromium\NativeMessagingHosts'
    Edge = 'HKCU:\Software\Microsoft\Edge\NativeMessagingHosts'
}
foreach ($name in $Browser) {
    $key = Join-Path $roots[$name] 'org.xdm.browser'
    New-Item -Force -Path $key | Out-Null
    Set-Item -Path $key -Value $manifestPath
    Write-Output "$name`: $key -> $manifestPath"
}
