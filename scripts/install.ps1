$ErrorActionPreference = 'Stop'

$repo = 'britonmearsty/hi'
$assetUrl = "https://github.com/$repo/releases/latest/download/hi-windows-x86_64.exe.zip"

$temp = Join-Path ([System.IO.Path]::GetTempPath()) ("hi-install-" + [guid]::NewGuid())
$binDir = Join-Path $env:LOCALAPPDATA 'Programs\hi'
New-Item -ItemType Directory -Force $temp, $binDir | Out-Null
$archive = Join-Path $temp 'hi.zip'
try {
    Invoke-WebRequest $assetUrl -OutFile $archive
} catch {
    throw "No Windows release is available yet. Install Rust, then run: cargo install --git https://github.com/$repo.git --locked"
}
Expand-Archive $archive -DestinationPath $temp -Force
$binary = Get-ChildItem $temp -Filter 'hi.exe' -Recurse | Select-Object -First 1
if (-not $binary) { throw 'Release archive did not contain hi.exe' }
Copy-Item $binary.FullName (Join-Path $binDir 'hi.exe') -Force
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$binDir*") { [Environment]::SetEnvironmentVariable('Path', "$userPath;$binDir", 'User') }
Write-Host "Installed hi to $binDir. Open a new terminal, then run: hi"
Remove-Item $temp -Recurse -Force
