# sniptool memory sampler - Spike 1 acceptance harness
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Launch -Seconds 120
#   powershell -ExecutionPolicy Bypass -File tools\measure.ps1 -Seconds 60      # attach to a running instance
#
# Columns:
#   WS_MB      working set   -> what Task Manager shows in the "Memory" column (the agreed 20MB budget)
#   PRIV_MB    private bytes -> pages owned exclusively by this process
#   PEAK_MB    peak working set since start
#   HND / THR / GDI          -> leak indicators
#
# NOTE: kept ASCII-only on purpose. Windows PowerShell 5.1 mis-decodes .ps1 files
# that contain non-ASCII text but no BOM.

param(
    [string]$ExePath = "",
    [int]$Seconds = 60,
    [int]$IntervalSeconds = 2,
    [switch]$Launch,
    [switch]$KeepRunning
)

$ErrorActionPreference = "Stop"

# ---- GDI object count via user32!GetGuiResources ---------------------------
try {
    if (-not ("SniptoolMem.Native" -as [type])) {
        Add-Type -Namespace SniptoolMem -Name Native -MemberDefinition @"
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern uint GetGuiResources(System.IntPtr hProcess, uint uiFlags);
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true)]
public static extern System.IntPtr OpenProcess(uint dwDesiredAccess, bool bInheritHandle, int dwProcessId);
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true)]
public static extern bool CloseHandle(System.IntPtr hObject);
"@
    }
} catch {
    Write-Host "WARN: could not compile GDI helper; GDI column will show -1" -ForegroundColor Yellow
}

function Get-GdiCount([int]$ProcessId) {
    if (-not ("SniptoolMem.Native" -as [type])) { return -1 }
    $h = [SniptoolMem.Native]::OpenProcess(0x0400, $false, $ProcessId)   # PROCESS_QUERY_INFORMATION
    if ($h -eq [System.IntPtr]::Zero) { return -1 }
    try   { return [int][SniptoolMem.Native]::GetGuiResources($h, 0) }  # GR_GDIOBJECTS
    finally { [void][SniptoolMem.Native]::CloseHandle($h) }
}

# ---- locate or launch ------------------------------------------------------
if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $root = Split-Path -Parent $PSScriptRoot
    $ExePath = Join-Path $root "target\release\sniptool.exe"
}

$proc = $null
if ($Launch) {
    if (-not (Test-Path $ExePath)) {
        throw "executable not found: $ExePath`nBuild it first:  cargo build --release"
    }
    $proc = Start-Process -FilePath $ExePath -PassThru
    Start-Sleep -Milliseconds 900
    Write-Host ("launched pid {0}  ({1})" -f $proc.Id, $ExePath) -ForegroundColor Cyan
} else {
    $proc = Get-Process -Name "sniptool" -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $proc) {
        throw "sniptool is not running. Start it first, or add -Launch"
    }
    Write-Host ("attached to pid {0}" -f $proc.Id) -ForegroundColor Cyan
}

# ---- sample ---------------------------------------------------------------
$samples = New-Object System.Collections.ArrayList
$clock = [System.Diagnostics.Stopwatch]::StartNew()

Write-Host ""
Write-Host ("{0,6} {1,7} {2,9} {3,9} {4,9} {5,6} {6,6} {7,5}" -f "T+sec", "WS_MB", "PRIV_MB", "PEAK_MB", "DELTA", "HND", "THR", "GDI")
Write-Host ("{0,6} {1,7} {2,9} {3,9} {4,9} {5,6} {6,6} {7,5}" -f "------", "-----", "-------", "-------", "-----", "---", "---", "---")

while ($clock.Elapsed.TotalSeconds -lt $Seconds) {
    $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
    if (-not $p) {
        Write-Host "process exited" -ForegroundColor Yellow
        break
    }
    $p.Refresh()

    $ws   = [math]::Round($p.WorkingSet64 / 1MB, 2)
    $priv = [math]::Round($p.PrivateMemorySize64 / 1MB, 2)
    $peak = [math]::Round($p.PeakWorkingSet64 / 1MB, 2)
    $gdi  = Get-GdiCount $proc.Id

    if ($samples.Count -gt 0) {
        $delta = [math]::Round($ws - $samples[0].WS, 2)
        $deltaTxt = "{0:+0.00;-0.00;0.00}" -f $delta
    } else {
        $delta = 0
        $deltaTxt = "0.00"
    }

    [void]$samples.Add([pscustomobject]@{
        T = [int]$clock.Elapsed.TotalSeconds; WS = $ws; Priv = $priv; Peak = $peak
        Hnd = $p.HandleCount; Thr = $p.Threads.Count; Gdi = $gdi
    })

    Write-Host ("{0,6} {1,7} {2,9} {3,9} {4,9} {5,6} {6,6} {7,5}" -f
        [int]$clock.Elapsed.TotalSeconds, $ws, $priv, $peak, $deltaTxt, $p.HandleCount, $p.Threads.Count, $gdi)

    Start-Sleep -Seconds $IntervalSeconds
}

# ---- summary --------------------------------------------------------------
if ($samples.Count -eq 0) {
    Write-Host "no samples collected" -ForegroundColor Red
    exit 1
}

$wsAll   = @($samples | ForEach-Object { $_.WS })
$privAll = @($samples | ForEach-Object { $_.Priv })
$first   = $samples[0]
$last    = $samples[$samples.Count - 1]
$wsMax   = ($wsAll | Measure-Object -Maximum).Maximum
$wsMin   = ($wsAll | Measure-Object -Minimum).Minimum
$wsAvg   = [math]::Round((($wsAll | Measure-Object -Average).Average), 2)
$drift   = [math]::Round($last.WS - $first.WS, 2)
$gdiOk   = $first.Gdi -ge 0
$gdiFirst = $first.Gdi
$gdiLast  = $last.Gdi
$hndDrift = $last.Hnd - $first.Hnd

Write-Host ""
Write-Host "==== summary ====" -ForegroundColor Cyan
Write-Host ("samples          : {0}  over {1}s" -f $samples.Count, $last.T)
Write-Host ("working set  min : {0} MB" -f $wsMin)
Write-Host ("working set  avg : {0} MB" -f $wsAvg)
Write-Host ("working set  max : {0} MB   <-- budget 20 MB" -f $wsMax)
Write-Host ("private      last: {0} MB" -f $last.Priv)
Write-Host ("peak (since start): {0} MB" -f $last.Peak)
Write-Host ("drift first->last: {0} MB" -f $drift)
Write-Host ("handles      delta: {0}" -f $hndDrift)
if ($gdiOk) { Write-Host ("GDI objects  delta: {0}" -f ($gdiLast - $gdiFirst)) }
Write-Host ""

$pass = $wsMax -le 20
if ($pass) {
    Write-Host ("RESULT: PASS  (max working set {0} MB <= 20 MB)" -f $wsMax) -ForegroundColor Green
} else {
    Write-Host ("RESULT: FAIL  (max working set {0} MB > 20 MB)" -f $wsMax) -ForegroundColor Red
}

if ([math]::Abs($drift) -gt 1.5) {
    Write-Host ("RESULT: WARN  working set drifted {0} MB during the run; check for a leak" -f $drift) -ForegroundColor Yellow
}
if ($hndDrift -gt 10) {
    Write-Host ("RESULT: WARN  handle count grew by {0}; check for a handle leak" -f $hndDrift) -ForegroundColor Yellow
}
if ($gdiOk -and (($gdiLast - $gdiFirst) -gt 5)) {
    Write-Host ("RESULT: WARN  GDI objects grew by {0}; check for a GDI leak" -f ($gdiLast - $gdiFirst)) -ForegroundColor Yellow
}

# ---- teardown -------------------------------------------------------------
if ($Launch -and -not $KeepRunning) {
    $p = Get-Process -Id $proc.Id -ErrorAction SilentlyContinue
    if ($p) {
        Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
        Write-Host "stopped pid $($proc.Id)"
    }
} else {
    Write-Host "sniptool left running (right-click the tray icon and choose Exit to stop it)"
}
