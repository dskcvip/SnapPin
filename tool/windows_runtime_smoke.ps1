param(
  [Parameter(Mandatory=$true)][string]$Executable,
  [Parameter(Mandatory=$true)][string]$ReportDirectory
)
$ErrorActionPreference = 'Stop'
$resolvedExe = (Resolve-Path -LiteralPath $Executable).Path
$report = [IO.Path]::GetFullPath($ReportDirectory)
New-Item -ItemType Directory -Path $report -Force | Out-Null
foreach ($mode in @('normal', 'shutdown')) {
  $profile = Join-Path $report $mode
  New-Item -ItemType Directory -Path $profile -Force | Out-Null
  $start = [Diagnostics.ProcessStartInfo]::new($resolvedExe)
  $start.WorkingDirectory = Split-Path -Parent $resolvedExe
  $start.UseShellExecute = $false
  $start.CreateNoWindow = $true
  $start.WindowStyle = 'Hidden'
  $start.EnvironmentVariables['APPDATA'] = $profile
  $start.EnvironmentVariables['SNAPPIN_RUNTIME_SMOKE'] = $mode
  $process = [Diagnostics.Process]::Start($start)
  if (-not $process.WaitForExit(60000)) {
    $process.Kill()
    throw "Runtime smoke timed out: $mode"
  }
  $exitCode = $process.ExitCode
  $process.Dispose()
  $logRoot = Join-Path $profile 'com.snappin.flutter\logs'
  $logs = (Get-ChildItem -LiteralPath $logRoot -Filter '*.log' | Get-Content) -join "`n"
  if ($exitCode -ne 0 -or $logs.Contains('[runtime-smoke-failed]')) {
    throw "Runtime smoke failed: $mode exit=$exitCode`n$logs"
  }
  $markers = if ($mode -eq 'normal') {
    @('runtime-smoke-shadow-ready', 'runtime-smoke-ocr-responsive',
      'runtime-smoke-ocr-busy-ready', 'runtime-smoke-ocr-layout-ready',
      'runtime-smoke-ocr-error-ready', '[runtime-smoke-ready]')
  } else { @('runtime-smoke-ocr-responsive', 'runtime-smoke-shutdown-start') }
  foreach ($marker in $markers) {
    if (-not $logs.Contains($marker)) { throw "Missing $mode runtime evidence: $marker" }
  }
  Write-Output "Release native runtime smoke passed: $mode (exit=$exitCode)"
}
