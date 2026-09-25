# Install Loadout's `lo` on Windows from a GitHub release, verified
# against the release's SHA256SUMS, then optionally run `lo init`.
#
#   irm https://raw.githubusercontent.com/Drewdl3/loadout/main/install.ps1 | iex
#   & ([scriptblock]::Create((irm …/install.ps1))) -CompanyConfig https://git.example.com/acme/agent-config
param(
  [string]$CompanyConfig = "",
  [string]$Version = "",
  [string]$Dir = "$env:LOCALAPPDATA\Programs\lo"
)
$ErrorActionPreference = "Stop"
$api = if ($env:LOADOUT_RELEASES_API) { $env:LOADOUT_RELEASES_API } else { "https://api.github.com/repos/Drewdl3/loadout" }
$headers = @{ "User-Agent" = "loadout-install" }
if ($env:GH_TOKEN) { $headers["Authorization"] = "Bearer $env:GH_TOKEN" }
$rel = if ($Version) { "$api/releases/tags/$Version" } else { "$api/releases/latest" }
$release = Invoke-RestMethod -Uri $rel -Headers $headers
$target = "x86_64-pc-windows-msvc"
$asset = $release.assets | Where-Object { $_.name -like "lo-*-$target.zip" } | Select-Object -First 1
$sums = $release.assets | Where-Object { $_.name -eq "SHA256SUMS" } | Select-Object -First 1
if (-not $asset -or -not $sums) { throw "The release has no Windows build or no SHA256SUMS." }
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("lo-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $zip = Join-Path $tmp $asset.name
  Invoke-WebRequest -Uri $asset.browser_download_url -Headers $headers -OutFile $zip
  $sumsText = (Invoke-WebRequest -Uri $sums.browser_download_url -Headers $headers).Content
  if ($sumsText -is [byte[]]) { $sumsText = [System.Text.Encoding]::UTF8.GetString($sumsText) }
  $expected = ($sumsText -split "`n" | Where-Object { $_ -match [regex]::Escape($asset.name) + '$' } | ForEach-Object { ($_ -split '\s+')[0] }) | Select-Object -First 1
  $actual = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
  if (-not $expected -or $expected.ToLower() -ne $actual) { throw "Checksum mismatch for $($asset.name); not installing." }
  Expand-Archive -Path $zip -DestinationPath $tmp
  $bin = Get-ChildItem -Path $tmp -Recurse -Filter lo.exe | Select-Object -First 1
  New-Item -ItemType Directory -Force -Path $Dir | Out-Null
  Copy-Item $bin.FullName (Join-Path $Dir "lo.exe") -Force
} finally { Remove-Item -Recurse -Force $tmp }
$exe = Join-Path $Dir "lo.exe"
Write-Host "Installed $(& $exe --version) to $exe"
$userPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not ($userPath -split ';' | Where-Object { $_ -eq $Dir })) {
  [Environment]::SetEnvironmentVariable("Path", "$userPath;$Dir", "User")
  Write-Host "Added $Dir to your user PATH (open a new terminal)."
}
if ($CompanyConfig) { & $exe init $CompanyConfig } else { Write-Host "Next: lo init <your company config URL>" }
