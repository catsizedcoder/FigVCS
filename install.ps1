$ErrorActionPreference = "Stop"

$repo = "catsizedcoder/FigVCS"
$installDir = Join-Path $env:LOCALAPPDATA "Programs\FigVCS"

Write-Host "Downloading the latest FigVCS release..."
$release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest"
$asset = $release.assets | Where-Object { $_.name -eq "fvcs-windows-x86_64.zip" } | Select-Object -First 1
if (-not $asset) {
    Write-Host "Could not find a Windows download in the latest release."
    exit 1
}

$zip = Join-Path $env:TEMP "fvcs.zip"
Invoke-WebRequest $asset.browser_download_url -OutFile $zip

New-Item -ItemType Directory -Force $installDir | Out-Null
Expand-Archive -Force $zip $installDir
Remove-Item $zip

$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($userPath -notlike "*$installDir*") {
    [Environment]::SetEnvironmentVariable("Path", "$userPath;$installDir", "User")
    Write-Host "Added FigVCS to your PATH."
}

Write-Host ""
Write-Host "FigVCS $($release.tag_name) installed to $installDir"
Write-Host "Open a NEW terminal (this is important!), then run: fvcs --help"
