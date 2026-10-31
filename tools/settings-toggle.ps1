# Settings round trip: toggle -> save -> config.ini + HKCU Run value.
#
# Proves the only self-drawn control that touches system state works end to end,
# in BOTH directions:
#   round 1  toggle OFF -> save -> config says autostart=0 AND the Run value is gone
#   round 2  toggle ON  -> save -> config says autostart=1 AND the Run value is back
#
# It clicks the real pixels (the whole window is GDI-drawn, there is no button
# control to send a message to), so it depends on the layout constants in
# src/ui/settings.rs: toggle at window-local (414,436), save at (408,518).
#
# Before touching anything it snapshots config.ini and the Run value, and it
# restores BOTH in a finally block -- including the "did not exist before" case,
# which means deleting the file / the registry value again. A previous version
# only restored what had existed, which left a permanent startup entry pointing
# at target\release\sniptool.exe on every machine that ran the test.
#
# Takes over the screen for a few seconds and kills any running sniptool.
# ASCII-only on purpose (PS 5.1 mis-decodes BOM-less non-ASCII .ps1).

param()

$ErrorActionPreference = "Stop"

if (-not ("Tt.Native" -as [type])) {
    Add-Type -Namespace Tt -Name Native -MemberDefinition @"
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode)]
public static extern System.IntPtr FindWindowW(string c, System.IntPtr w);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool PostMessageW(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool GetWindowRect(System.IntPtr h, out RECT r);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern void mouse_event(uint f, int dx, int dy, uint d, System.IntPtr e);
public struct RECT { public int Left, Top, Right, Bottom; }
"@
}
Add-Type -AssemblyName System.Windows.Forms

$MOVE = 0x0001; $ABS = 0x8000; $LDOWN = 0x0002; $LUP = 0x0004
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds

function Move-To([int]$x, [int]$y) {
    [Tt.Native]::mouse_event(($MOVE -bor $ABS), [int]($x * 65535 / ($b.Width - 1)), [int]($y * 65535 / ($b.Height - 1)), 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 70
}
function Click-At([int]$x, [int]$y) {
    Move-To $x $y
    [Tt.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 50
    [Tt.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 200
}

# ---- snapshot ------------------------------------------------------------
$cfg = Join-Path $env:APPDATA "MySnipTool\config.ini"
$bak = "$cfg.tbak"
$hadCfg = Test-Path $cfg
if ($hadCfg) { Copy-Item $cfg $bak -Force }

$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$prevRun = (Get-ItemProperty -Path $runKey -Name "MySnipTool" -ErrorAction SilentlyContinue).MySnipTool
$hadRun = $null -ne $prevRun

$proc = $null
function Restore-State {
    if ($null -ne $script:proc) {
        Stop-Process -Id $script:proc.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 300
    }
    if ($hadCfg) {
        if (Test-Path $bak) { Copy-Item $bak $cfg -Force; Remove-Item $bak -Force }
    } else {
        # there was no config before: remove the one this test created, so the
        # next start goes back to the built-in defaults
        Remove-Item $cfg -Force -ErrorAction SilentlyContinue
    }
    if ($hadRun) {
        Set-ItemProperty -Path $runKey -Name "MySnipTool" -Value $prevRun
    } else {
        # there was no startup entry before: remove the one this test created
        Remove-ItemProperty -Path $runKey -Name "MySnipTool" -ErrorAction SilentlyContinue
    }
    Write-Host "restored config and Run value"
}

trap {
    Write-Host ("ABORTED: {0}" -f $_.Exception.Message) -ForegroundColor Red
    Restore-State
    exit 1
}

# ---- run -----------------------------------------------------------------
$exe = Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\sniptool.exe"
if (-not (Test-Path $exe)) { throw "not found: $exe" }

Get-Process -Name sniptool -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400
$proc = Start-Process $exe -PassThru
Start-Sleep -Milliseconds 1300

function Open-Settings {
    $hostWin = [Tt.Native]::FindWindowW("MySnipTool.TrayHost", [System.IntPtr]::Zero)
    if ($hostWin -eq [System.IntPtr]::Zero) { throw "tray host not found" }
    [void][Tt.Native]::PostMessageW($hostWin, 0x0111, [System.IntPtr]1007, [System.IntPtr]::Zero)
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 100
        $w = [Tt.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero)
        if ($w -ne [System.IntPtr]::Zero) { return $w }
    }
    throw "settings window did not open"
}

# toggle, then save; returns nothing, throws if the window never closes
function Toggle-And-Save([System.IntPtr]$Win) {
    $r = New-Object Tt.Native+RECT
    [void][Tt.Native]::GetWindowRect($Win, [ref]$r)
    Click-At ($r.Left + 414) ($r.Top + 436)     # the sliding toggle
    Start-Sleep -Milliseconds 400
    Click-At ($r.Left + 408) ($r.Top + 518)     # the blue save button
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Milliseconds 100
        if ([Tt.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero) -eq [System.IntPtr]::Zero) { return }
    }
    throw "save button did not close the window (click missed?)"
}

$w = Open-Settings
$r = New-Object Tt.Native+RECT
[void][Tt.Native]::GetWindowRect($w, [ref]$r)
Write-Host ("window rect {0},{1} {2}x{3}" -f $r.Left, $r.Top, ($r.Right - $r.Left), ($r.Bottom - $r.Top))
$before = if (Test-Path $cfg) { (Select-String -Path $cfg -Pattern "^autostart=").Line } else { "none" }
Write-Host "config before : $before"
Write-Host ("run key before: {0}" -f $(if ($prevRun) { "present" } else { "absent" }))

# ---- round 1: off --------------------------------------------------------
Toggle-And-Save $w
$after = if (Test-Path $cfg) { (Select-String -Path $cfg -Pattern "^autostart=").Line } else { "none" }
$runAfter = (Get-ItemProperty -Path $runKey -Name "MySnipTool" -ErrorAction SilentlyContinue).MySnipTool
Write-Host "config after  : $after"
Write-Host ("run key after : {0}" -f $(if ($runAfter) { "present" } else { "absent" }))
if (($after -match "autostart=0") -and ($null -eq $runAfter)) {
    Write-Host "PASS  toggle off -> config=0 and Run value removed"
} else {
    Write-Host "FAIL  toggle off did not persist" -ForegroundColor Red
}

# ---- round 2: back on ----------------------------------------------------
$w = Open-Settings
Toggle-And-Save $w
$cfg2 = (Select-String -Path $cfg -Pattern "^autostart=").Line
$run2 = (Get-ItemProperty -Path $runKey -Name "MySnipTool" -ErrorAction SilentlyContinue).MySnipTool
Write-Host "config round2 : $cfg2"
Write-Host ("run key round2: {0}" -f $(if ($run2) { "present" } else { "absent" }))
if (($cfg2 -match "autostart=1") -and ($null -ne $run2)) {
    Write-Host "PASS  toggle on -> config=1 and Run value written"
} else {
    Write-Host "FAIL  toggle on did not persist" -ForegroundColor Red
}

Restore-State
