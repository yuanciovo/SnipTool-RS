# Ghost-text regression probe (deterministic, no racing required)
#
# The bug: the size label is anchored at sel.left and can be wider than the
# selection. If the dirty rect is derived from the selection only, the label's
# tail falls outside it, gets clipped while painting, and is never erased when
# the selection shrinks. Result: the old "455 x 260  drag inside..." line stays
# on screen and looks stretched / doubled.
#
# How this probe isolates it:
#   1. frozen mode -> the backdrop is a static bitmap, so nothing behind us moves
#   2. draw a WIDE selection (label paints fully), then shrink it with the E
#      handle (dirty rect becomes narrow)
#   3. screenshot A
#   4. RedrawWindow(RDW_INVALIDATE|RDW_ERASE|RDW_UPDATENOW) forces a full repaint
#   5. screenshot B
#
# A and B must be pixel identical. Any difference is stale paint left behind.
#
# ASCII-only on purpose (PS 5.1 mis-decodes BOM-less non-ASCII .ps1).

param(
    [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"

if (-not ("G5.Native" -as [type])) {
    Add-Type -Namespace G5 -Name Native -MemberDefinition @"
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
public static extern System.IntPtr FindWindowW(string c, System.IntPtr w);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern bool PostMessageW(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
public static extern int GetWindowTextW(System.IntPtr h, System.Text.StringBuilder s, int n);
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
public static extern System.IntPtr FindWindowExW(System.IntPtr p, System.IntPtr a, string c, System.IntPtr w);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern bool RedrawWindow(System.IntPtr h, System.IntPtr r, System.IntPtr rgn, uint flags);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern void mouse_event(uint f, int dx, int dy, uint d, System.IntPtr e);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern System.IntPtr GetDC(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern int ReleaseDC(System.IntPtr h, System.IntPtr hdc);
[System.Runtime.InteropServices.DllImport("gdi32.dll", SetLastError=true)]
public static extern int GetDeviceCaps(System.IntPtr hdc, int index);
"@
}

$MOVE = 0x0001; $ABS = 0x8000; $LDOWN = 0x0002; $LUP = 0x0004
$WM_HOTKEY = 0x0312
$RDW_INVALIDATE = 0x0001; $RDW_ERASE = 0x0004; $RDW_UPDATENOW = 0x0100

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$hdc = [G5.Native]::GetDC([System.IntPtr]::Zero)
$physW = [G5.Native]::GetDeviceCaps($hdc, 118)
$physH = [G5.Native]::GetDeviceCaps($hdc, 119)
[void][G5.Native]::ReleaseDC([System.IntPtr]::Zero, $hdc)
$scale = if ($physW -gt 0) { $physW / $b.Width } else { 1 }
Write-Host ("INFO  screen {0}x{1} phys, {2}x{3} virtual, scale {4}" -f $physW, $physH, $b.Width, $b.Height, $scale) -ForegroundColor Cyan

function Move-To([int]$x, [int]$y) {
    $nx = [int]($x * 65535 / ($b.Width - 1))
    $ny = [int]($y * 65535 / ($b.Height - 1))
    [G5.Native]::mouse_event(($MOVE -bor $ABS), $nx, $ny, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 30
}

function Title-Of([System.IntPtr]$h) {
    if ($h -eq [System.IntPtr]::Zero) { return "<none>" }
    $sb = New-Object System.Text.StringBuilder 160
    [void][G5.Native]::GetWindowTextW($h, $sb, 160)
    return $sb.ToString()
}

function Snap([string]$path) {
    $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen(0, 0, 0, 0, $bmp.Size)
    $g.Dispose()
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
}

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\sniptool.exe"
}
if (-not (Test-Path $ExePath)) { throw "not found: $ExePath" }

Get-Process -Name "sniptool" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400
$proc = Start-Process -FilePath $ExePath -PassThru
Start-Sleep -Milliseconds 1200

$host_ = [G5.Native]::FindWindowW("MySnipTool.TrayHost", [System.IntPtr]::Zero)
if ($host_ -eq [System.IntPtr]::Zero) { throw "tray host not found" }

# frozen mode: static backdrop, so any pixel difference is our own doing
[void][G5.Native]::PostMessageW($host_, $WM_HOTKEY, [System.IntPtr]2, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 1600

$ov = [G5.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
if ($ov -eq [System.IntPtr]::Zero) { throw "overlay did not open" }

# 1) a NARROW selection -- narrower than the label text, so the tail of the
#    hint sticks out past the selection and past the selection-based dirty rect
$ax = [int]($b.Width * 0.30); $ay = [int]($b.Height * 0.30)
$bx = [int]($b.Width * 0.38); $by = [int]($b.Height * 0.44)
Move-To $ax $ay
[G5.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
for ($i = 1; $i -le 6; $i++) {
    Move-To ($ax + [int](($bx - $ax) * $i / 6)) ($ay + [int](($by - $ay) * $i / 6))
}
[G5.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 700
$ov = [G5.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
Write-Host ("INFO  after drag: overlay='{0}' toolbar={1}" -f (Title-Of $ov), ([G5.Native]::FindWindowW("MySnipTool.Toolbar", [System.IntPtr]::Zero) -ne [System.IntPtr]::Zero)) -ForegroundColor DarkGray

# 2) drag the selection from its centre. The catcher window forwards the click,
#    so this is the one proven input path; and because the whole label slides
#    with the selection, the previous label position is guaranteed to fall
#    outside a dirty rect derived from the selection alone.
$cx = [int](($ax + $bx) / 2)
$midy = [int](($ay + $by) / 2)
Move-To $cx $midy
Start-Sleep -Milliseconds 120
[G5.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
$dx = [int]($b.Width * 0.10)
for ($i = 1; $i -le 8; $i++) {
    Move-To ($cx + [int]($dx * $i / 8)) ($midy + [int]($dx * $i / 8 * 0.4))
}
$ov = [G5.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
Write-Host ("INFO  during drag: overlay='{0}'" -f (Title-Of $ov)) -ForegroundColor DarkGray
[G5.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 500
$ov = [G5.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
Write-Host ("INFO  after move:   overlay='{0}'" -f (Title-Of $ov)) -ForegroundColor DarkGray

# 3) capture, force a full repaint, capture again
$tmp = Join-Path $env:TEMP "ghost5"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$a = Join-Path $tmp "after_shrink.png"
$c = Join-Path $tmp "after_redraw.png"
Snap $a
[void][G5.Native]::RedrawWindow($ov, [System.IntPtr]::Zero, [System.IntPtr]::Zero, ($RDW_INVALIDATE -bor $RDW_ERASE -bor $RDW_UPDATENOW))
Start-Sleep -Milliseconds 350
Snap $c

# 4) compare. Hash first (instant); only dig into pixels when they differ,
#    because GetPixel over a million pixels takes minutes in PowerShell.
function Get-Hash($bmp) {
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Bmp)
    $bytes = $ms.ToArray()
    $ms.Dispose()
    return [System.BitConverter]::ToString([System.Security.Cryptography.MD5]::Create().ComputeHash($bytes))
}

$ia = [System.Drawing.Image]::FromFile($a)
$ic = [System.Drawing.Image]::FromFile($c)
$ba = New-Object System.Drawing.Bitmap $ia
$bc = New-Object System.Drawing.Bitmap $ic

if ((Get-Hash $ba) -eq (Get-Hash $bc)) {
    Write-Host "PASS  no stale paint after shrink (pixel identical)" -ForegroundColor Green
} else {
    # coarse 4x4 block scan just to report where the residue is
    $minX = 99999; $maxX = -1; $minY = 99999; $maxY = -1; $blocks = 0
    for ($y = 0; $y -lt $ba.Height; $y += 4) {
        for ($x = 0; $x -lt $ba.Width; $x += 4) {
            $pa = $ba.GetPixel($x, $y)
            $pc = $bc.GetPixel($x, $y)
            if ($pa.R -ne $pc.R -or $pa.G -ne $pc.G -or $pa.B -ne $pc.B) {
                $blocks++
                if ($x -lt $minX) { $minX = $x }
                if ($x -gt $maxX) { $maxX = $x }
                if ($y -lt $minY) { $minY = $y }
                if ($y -gt $maxY) { $maxY = $y }
            }
        }
    }
    Write-Host ("FAIL  stale paint: {0} blocks differ, bbox x {1}..{2} y {3}..{4}" -f $blocks, $minX, $maxX, $minY, $maxY) -ForegroundColor Red
}
$ba.Dispose(); $bc.Dispose(); $ia.Dispose(); $ic.Dispose()
Write-Host ("INFO  shots: {0}" -f $tmp) -ForegroundColor DarkGray

Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
