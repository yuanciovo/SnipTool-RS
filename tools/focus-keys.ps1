# probe: (a) does the settings window survive losing focus?
#        (b) does a REAL Enter key confirm a frozen capture?
param()

$ErrorActionPreference = "Stop"
if (-not ("Pb.Native" -as [type])) {
    Add-Type -Namespace Pb -Name Native -MemberDefinition @"
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode)]
public static extern System.IntPtr FindWindowW(string c, System.IntPtr w);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool PostMessageW(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool GetWindowRect(System.IntPtr h, out RECT r);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern System.IntPtr GetForegroundWindow();
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool IsWindow(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern bool IsWindowVisible(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern void mouse_event(uint f, int dx, int dy, uint d, System.IntPtr e);
[System.Runtime.InteropServices.DllImport("user32.dll")]
public static extern void keybd_event(byte vk, byte scan, uint flags, System.IntPtr extra);
public struct RECT { public int Left, Top, Right, Bottom; }
"@
}
Add-Type -AssemblyName System.Windows.Forms
$MOVE = 0x0001; $ABS = 0x8000; $LDOWN = 0x0002; $LUP = 0x0004
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds

function Move-To([int]$x, [int]$y) {
    [Pb.Native]::mouse_event(($MOVE -bor $ABS), [int]($x * 65535 / ($b.Width - 1)), [int]($y * 65535 / ($b.Height - 1)), 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 60
}
function Click-At([int]$x, [int]$y) {
    Move-To $x $y
    [Pb.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 40
    [Pb.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
}

$exe = Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\sniptool.exe"
Get-Process -Name sniptool -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400
$p = Start-Process $exe -PassThru
Start-Sleep -Milliseconds 1300
$tray = [Pb.Native]::FindWindowW("MySnipTool.TrayHost", [System.IntPtr]::Zero)

# ---------- (a) settings window + focus loss ----------
[void][Pb.Native]::PostMessageW($tray, 0x0111, [System.IntPtr]1007, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 900
$w = [Pb.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero)
Write-Output ("settings hwnd = 0x{0:X}" -f $w.ToInt64())

# click somewhere far away (the desktop) to move the focus elsewhere
Click-At 60 700
Start-Sleep -Milliseconds 500
$alive = [Pb.Native]::IsWindow($w)
$visible = [Pb.Native]::IsWindowVisible($w)
$fg = [Pb.Native]::GetForegroundWindow()
Write-Output ("after clicking outside: isWindow={0} visible={1} foreground==settings: {2}" -f $alive, $visible, ($fg -eq $w))

# ---------- (b) frozen capture + real Enter ----------
if ($alive) { [void][Pb.Native]::PostMessageW($w, 0x0010, [System.IntPtr]::Zero, [System.IntPtr]::Zero) }
Start-Sleep -Milliseconds 400

$dir = Join-Path $env:TEMP "MySnipTool"
$before = @(Get-ChildItem $dir -Filter "snip_*.png" -ErrorAction SilentlyContinue).Count

[void][Pb.Native]::PostMessageW($tray, 0x0312, [System.IntPtr]2, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 1500
$ov = [Pb.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
Write-Output ("overlay hwnd = 0x{0:X}  foreground==overlay: {1}" -f $ov.ToInt64(), ([Pb.Native]::GetForegroundWindow() -eq $ov))

# Esc without any drag: proves the overlay grabbed the keyboard right away
[Pb.Native]::keybd_event(0x1B, 0, 0, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 60
[Pb.Native]::keybd_event(0x1B, 0, 2, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 700
$escNoDrag = [Pb.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero) -eq [System.IntPtr]::Zero
if ($escNoDrag) { Write-Output "PASS  Esc works before any drag" } else { Write-Output "FAIL  Esc without drag does nothing" }

if (-not $escNoDrag) { [void][Pb.Native]::PostMessageW($ov, 0x0010, [System.IntPtr]::Zero, [System.IntPtr]::Zero); Start-Sleep -Milliseconds 300 }

# reopen for the drag + Enter test
[void][Pb.Native]::PostMessageW($tray, 0x0312, [System.IntPtr]2, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 1500
$ov = [Pb.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)

$fx = [int]($b.Width * 0.28); $fy = [int]($b.Height * 0.28)
$tx = [int]($b.Width * 0.62); $ty = [int]($b.Height * 0.62)
Move-To $fx $fy
[Pb.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
for ($i = 1; $i -le 8; $i++) {
    Move-To ($fx + [int](($tx - $fx) * $i / 8)) ($fy + [int](($ty - $fy) * $i / 8))
}
[Pb.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 700

$pending = [Pb.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero) -ne [System.IntPtr]::Zero
$bar = [Pb.Native]::FindWindowW("MySnipTool.Toolbar", [System.IntPtr]::Zero) -ne [System.IntPtr]::Zero
$fg2 = [Pb.Native]::GetForegroundWindow()
Write-Output ("pending={0} toolbar={1} foreground==overlay: {2}" -f $pending, $bar, ($fg2 -eq $ov))

# real Enter
[Pb.Native]::keybd_event(0x0D, 0, 0, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 60
[Pb.Native]::keybd_event(0x0D, 0, 2, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 1200

$after = @(Get-ChildItem $dir -Filter "snip_*.png" -ErrorAction SilentlyContinue).Count
$ovGone = [Pb.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero) -eq [System.IntPtr]::Zero
Write-Output ("after real Enter: png {0} -> {1}, overlayGone={2}" -f $before, $after, $ovGone)
if ($after -eq ($before + 1)) { Write-Output "PASS  real Enter confirms" } else { Write-Output "FAIL  real Enter does nothing" }

# real Esc path
[void][Pb.Native]::PostMessageW($tray, 0x0312, [System.IntPtr]2, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 1500
$ov2 = [Pb.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
if ($ov2 -ne [System.IntPtr]::Zero) {
    Move-To $fx $fy
    [Pb.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
    for ($i = 1; $i -le 6; $i++) {
        Move-To ($fx + [int](($tx - $fx) * $i / 6)) ($fy + [int](($ty - $fy) * $i / 6))
    }
    [Pb.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 600
    [Pb.Native]::keybd_event(0x1B, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [Pb.Native]::keybd_event(0x1B, 0, 2, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 700
    $gone = [Pb.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero) -eq [System.IntPtr]::Zero
    if ($gone) { Write-Output "PASS  real Esc cancels" } else { Write-Output "FAIL  real Esc does nothing" }
}

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
