param(
  [string]$Destination
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $Destination) {
  $Destination = Join-Path $projectRoot ('dist\windows-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
}
$packageRoot = [IO.Path]::GetFullPath($Destination)
$runnerRoot = Join-Path $projectRoot 'build\windows\x64\runner\Release'
$aotSource = Join-Path $projectRoot 'build\windows\app.so'
$assetsSource = Join-Path $projectRoot 'build\flutter_assets'
foreach ($required in @((Join-Path $runnerRoot 'snappin_flutter.exe'), $aotSource, $assetsSource)) {
  if (-not (Test-Path -LiteralPath $required)) { throw "Missing compiled output: $required" }
}
if (Test-Path -LiteralPath $packageRoot) { throw "Choose a new package directory: $packageRoot" }
$dataRoot = Join-Path $packageRoot 'data'
New-Item -ItemType Directory -Path $dataRoot -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $runnerRoot 'snappin_flutter.exe') -Destination $packageRoot
Get-ChildItem -LiteralPath $runnerRoot -Filter '*.dll' -File | Copy-Item -Destination $packageRoot
# A running app may lock its installed DLLs and AOT image. Package the newly
# compiled plugin DLLs and app.so directly, without stopping the running app.
Get-ChildItem -LiteralPath (Join-Path $projectRoot 'build\windows\x64\plugins') -Recurse -Filter '*.dll' -File |
  Where-Object { $_.Directory.Name -eq 'Release' } |
  Copy-Item -Destination $packageRoot -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'windows\flutter\ephemeral\flutter_windows.dll') -Destination $packageRoot -Force
Copy-Item -LiteralPath (Join-Path $projectRoot 'windows\flutter\ephemeral\icudtl.dat') -Destination $dataRoot
Copy-Item -LiteralPath $assetsSource -Destination $dataRoot -Recurse
Copy-Item -LiteralPath $aotSource -Destination $dataRoot
$nativeAssets = Join-Path $projectRoot 'build\native_assets\windows'
if (Test-Path -LiteralPath $nativeAssets) {
  Get-ChildItem -LiteralPath $nativeAssets | Copy-Item -Destination $packageRoot -Recurse -Force
}
foreach ($source in @($aotSource, (Join-Path $projectRoot 'build\windows\x64\plugins\hotkey_manager_windows\Release\hotkey_manager_windows_plugin.dll'))) {
  $target = if ($source -eq $aotSource) { Join-Path $dataRoot 'app.so' } else { Join-Path $packageRoot 'hotkey_manager_windows_plugin.dll' }
  if ((Get-FileHash -LiteralPath $source).Hash -ne (Get-FileHash -LiteralPath $target).Hash) {
    throw "Package verification failed: $target"
  }
}
$manifest = Get-ChildItem -LiteralPath $packageRoot -Recurse -File | ForEach-Object {
  [ordered]@{
    file = [IO.Path]::GetRelativePath($packageRoot, $_.FullName)
    sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
  }
}
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $packageRoot 'manifest.json') -Encoding utf8
Write-Output (Join-Path $packageRoot 'snappin_flutter.exe')
