# temporary: toggle -> save -> config/registry round trip
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

$cfg = Join-Path $env:APPDATA "MySnipTool\config.ini"
$bak = "$cfg.tbak"
$had = Test-Path $cfg
if ($had) { Copy-Item $cfg $bak -Force }
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$prevRun = (Get-ItemProperty -Path $runKey -Name "MySnipTool" -ErrorAction SilentlyContinue).MySnipTool

$exe = Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\sniptool.exe"
Get-Process -Name sniptool -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400
$p = Start-Process $exe -PassThru
Start-Sleep -Milliseconds 1300

$host_ = [Tt.Native]::FindWindowW("MySnipTool.TrayHost", [System.IntPtr]::Zero)
[void][Tt.Native]::PostMessageW($host_, 0x0111, [System.IntPtr]1007, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 900
$w = [Tt.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero)
if ($w -eq [System.IntPtr]::Zero) { Write-Output "no window"; exit 1 }
$r = New-Object Tt.Native+RECT
[void][Tt.Native]::GetWindowRect($w, [ref]$r)

Write-Output ("window rect {0},{1} {2}x{3}" -f $r.Left, $r.Top, ($r.Right-$r.Left), ($r.Bottom-$r.Top))
$before = if (Test-Path $cfg) { (Select-String -Path $cfg -Pattern "^autostart=").Line } else { "none" }
Write-Output "config before : $before"
Write-Output ("run key before: {0}" -f $(if ($prevRun) { "present" } else { "absent" }))

# toggle at window-local (414,436), then save at (408,518)
Click-At ($r.Left + 414) ($r.Top + 436)
Start-Sleep -Milliseconds 400
$alive = [Tt.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero) -ne [System.IntPtr]::Zero
Write-Output ("after toggle click: window alive={0}" -f $alive)
Click-At ($r.Left + 408) ($r.Top + 518)
Start-Sleep -Milliseconds 1200
$stillOpen = [Tt.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero) -ne [System.IntPtr]::Zero
Write-Output ("after save click:   window alive={0}" -f $stillOpen)

$after = if (Test-Path $cfg) { (Select-String -Path $cfg -Pattern "^autostart=").Line } else { "none" }
$runAfter = (Get-ItemProperty -Path $runKey -Name "MySnipTool" -ErrorAction SilentlyContinue).MySnipTool
Write-Output "config after  : $after"
Write-Output ("run key after : {0}" -f $(if ($runAfter) { "present" } else { "absent" }))

$ok = ($after -match "autostart=0") -and ($null -eq $runAfter)
if ($ok) { Write-Output "PASS  toggle off -> config=0 and Run value removed" }
else { Write-Output "FAIL  toggle off did not persist" }

# round 2: turn it back on
[void][Tt.Native]::PostMessageW($host_, 0x0111, [System.IntPtr]1007, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 900
$w = [Tt.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero)
if ($w -ne [System.IntPtr]::Zero) {
    [void][Tt.Native]::GetWindowRect($w, [ref]$r)
    Click-At ($r.Left + 414) ($r.Top + 436)
    Start-Sleep -Milliseconds 400
    Click-At ($r.Left + 408) ($r.Top + 518)
    Start-Sleep -Milliseconds 1200
}
$cfg2 = (Select-String -Path $cfg -Pattern "^autostart=").Line
$run2 = (Get-ItemProperty -Path $runKey -Name "MySnipTool" -ErrorAction SilentlyContinue).MySnipTool
Write-Output "config round2 : $cfg2"
Write-Output ("run key round2: {0}" -f $(if ($run2) { "present" } else { "absent" }))
if (($cfg2 -match "autostart=1") -and ($null -ne $run2)) { Write-Output "PASS  toggle on -> config=1 and Run value written" }
else { Write-Output "FAIL  toggle on did not persist" }

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400
if ($had) { Copy-Item $bak $cfg -Force; Remove-Item $bak -Force }
if ($null -ne $prevRun) { Set-ItemProperty -Path $runKey -Name "MySnipTool" -Value $prevRun }
Write-Output "restored"
