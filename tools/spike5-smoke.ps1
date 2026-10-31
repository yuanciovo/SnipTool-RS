# Spike 5 smoke test: dynamic quick-capture, config file, settings window, autostart
#
# What it proves:
#   1. Alt+A (dynamic) writes the png and the clipboard on mouse-UP, with NO
#      pending state and NO toolbar window ever appearing
#   2. Ctrl+Alt+A (frozen) still goes through pending + toolbar + Enter
#   3. the config file is parsed (a non-default combo survives a save round-trip)
#   4. the settings window opens (one native EDIT inside) and the self-drawn
#      save button closes it when clicked
#   5. the Run registry value points at this exe
#
# It backs up config.ini and the Run value first and restores both at the end.
# Takes over the screen for ~6s and moves the mouse.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File tools\spike5-smoke.ps1
#
# ASCII-only on purpose (PS 5.1 mis-decodes BOM-less non-ASCII .ps1).

param(
    [switch]$KeepRunning,
    [string]$ExePath = ""
)

$ErrorActionPreference = "Stop"

if (-not ("S5.Native" -as [type])) {
    Add-Type -Namespace S5 -Name Native -MemberDefinition @"
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
public static extern System.IntPtr FindWindowW(string c, System.IntPtr w);
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
public static extern System.IntPtr FindWindowExW(System.IntPtr p, System.IntPtr after, string c, System.IntPtr w);
[System.Runtime.InteropServices.DllImport("user32.dll", CharSet=System.Runtime.InteropServices.CharSet.Unicode, SetLastError=true)]
public static extern int GetWindowTextW(System.IntPtr h, System.Text.StringBuilder s, int n);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern bool PostMessageW(System.IntPtr h, uint m, System.IntPtr w, System.IntPtr l);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern bool GetWindowRect(System.IntPtr h, out RECT r);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern void mouse_event(uint f, int dx, int dy, uint d, System.IntPtr e);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern uint GetDpiForSystem();
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern long GetWindowLongPtrW(System.IntPtr h, int index);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern System.IntPtr GetDC(System.IntPtr h);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern int ReleaseDC(System.IntPtr h, System.IntPtr hdc);
[System.Runtime.InteropServices.DllImport("gdi32.dll", SetLastError=true)]
public static extern int GetDeviceCaps(System.IntPtr hdc, int index);
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true)]
public static extern uint GetGuiResources(System.IntPtr h, uint f);
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true)]
public static extern System.IntPtr OpenProcess(uint a, bool i, int p);
[System.Runtime.InteropServices.DllImport("kernel32.dll", SetLastError=true)]
public static extern bool CloseHandle(System.IntPtr h);
public struct RECT { public int Left, Top, Right, Bottom; }
"@
}

$MOVE = 0x0001; $ABS = 0x8000; $LDOWN = 0x0002; $LUP = 0x0004
$WM_HOTKEY = 0x0312; $WM_KEYDOWN = 0x0100; $WM_COMMAND = 0x0111; $WM_CLOSE = 0x0010
$VK_RETURN = 0x0D
$IDM_SETTINGS = 1007

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$fx = [int]($b.Width * 0.28); $fy = [int]($b.Height * 0.28)
$tx = [int]($b.Width * 0.62); $ty = [int]($b.Height * 0.62)

function Move-To([int]$x, [int]$y) {
    $nx = [int]($x * 65535 / ($b.Width - 1))
    $ny = [int]($y * 65535 / ($b.Height - 1))
    [S5.Native]::mouse_event(($MOVE -bor $ABS), $nx, $ny, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 40
}

function Get-WsMb([int]$ProcessId) {
    $p = Get-Process -Id $ProcessId -ErrorAction SilentlyContinue
    if (-not $p) { return -1 }
    return [math]::Round($p.WorkingSet64 / 1MB, 2)
}

function Get-Gdi([int]$ProcessId) {
    $h = [S5.Native]::OpenProcess(0x0400, $false, $ProcessId)
    if ($h -eq [System.IntPtr]::Zero) { return -1 }
    try { return [int][S5.Native]::GetGuiResources($h, 0) }
    finally { [void][S5.Native]::CloseHandle($h) }
}

function Show-Result([string]$Name, [bool]$Ok, [string]$Detail) {
    if ($Ok) { Write-Host ("PASS  {0,-34} {1}" -f $Name, $Detail) -ForegroundColor Green }
    else     { Write-Host ("FAIL  {0,-34} {1}" -f $Name, $Detail) -ForegroundColor Red }
}

function Count-Files($dir, $pattern) {
    return @(Get-ChildItem $dir -Filter $pattern -ErrorAction SilentlyContinue).Count
}

function Title-Of([System.IntPtr]$h) {
    if ($h -eq [System.IntPtr]::Zero) { return "" }
    $sb = New-Object System.Text.StringBuilder 200
    [void][S5.Native]::GetWindowTextW($h, $sb, 200)
    return $sb.ToString()
}

function Count-ChildClass([System.IntPtr]$parent, [string]$class) {
    $n = 0
    $child = [System.IntPtr]::Zero
    while ($true) {
        $child = [S5.Native]::FindWindowExW($parent, $child, $class, [System.IntPtr]::Zero)
        if ($child -eq [System.IntPtr]::Zero) { break }
        $n++
    }
    return $n
}

if ([string]::IsNullOrWhiteSpace($ExePath)) {
    $ExePath = Join-Path (Split-Path -Parent $PSScriptRoot) "target\release\sniptool.exe"
}
if (-not (Test-Path $ExePath)) { throw "not found: $ExePath" }

$dir = Join-Path $env:TEMP "MySnipTool"
New-Item -ItemType Directory -Force -Path $dir | Out-Null

$cfgPath = Join-Path $env:APPDATA "MySnipTool\config.ini"
$cfgBak = "$cfgPath.bak"
$runKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$valueName = "MySnipTool"

# ---- backup --------------------------------------------------------------
Get-Process -Name "sniptool" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

$hadCfg = Test-Path $cfgPath
if ($hadCfg) { Copy-Item $cfgPath $cfgBak -Force }
$prevRun = (Get-ItemProperty -Path $runKey -Name $valueName -ErrorAction SilentlyContinue).$valueName

# Restores both sides of what this test touches. Note the two "did not exist
# before" branches: they DELETE, because leaving a config.ini or a startup entry
# behind changes the tester's machine for good (a Run value pointing at
# target\release\sniptool.exe survives cargo clean and fires at every logon).
function Restore-State {
    if ($null -ne $script:proc) {
        Stop-Process -Id $script:proc.Id -Force -ErrorAction SilentlyContinue
        Start-Sleep -Milliseconds 400
    }
    if ($hadCfg) {
        if (Test-Path $cfgBak) { Copy-Item $cfgBak $cfgPath -Force; Remove-Item $cfgBak -Force }
    } else {
        Remove-Item $cfgPath -Force -ErrorAction SilentlyContinue
    }
    if ($null -ne $prevRun) {
        Set-ItemProperty -Path $runKey -Name $valueName -Value $prevRun
    } else {
        Remove-ItemProperty -Path $runKey -Name $valueName -ErrorAction SilentlyContinue
    }
    Write-Host "restored config and Run value"
}

# any terminating error (assertion blown inside a helper, window never opened,
# process crashed) still has to put the machine back the way it was
trap {
    Write-Host ("ABORTED: {0}" -f $_.Exception.Message) -ForegroundColor Red
    Restore-State
    exit 1
}

# ---- seed a config with a NON-default history combo ----------------------
# If the app silently fell back to defaults, the save round-trip below would
# rewrite it as Alt+V and this assertion would catch it.
New-Item -ItemType Directory -Force -Path (Split-Path $cfgPath) | Out-Null
$seed = @"
# seeded by spike5-smoke.ps1
quick=Ctrl+Shift+F9
frozen=Ctrl+Alt+A
history=Alt+Shift+V
retention_hours=2
autostart=1
"@
Set-Content -Path $cfgPath -Value $seed -Encoding ASCII

# ---- start ---------------------------------------------------------------
$proc = Start-Process -FilePath $ExePath -PassThru
Start-Sleep -Milliseconds 1300

$trayHost = [S5.Native]::FindWindowW("MySnipTool.TrayHost", [System.IntPtr]::Zero)
if ($trayHost -eq [System.IntPtr]::Zero) {
    Write-Host "FAIL  tray host not found" -ForegroundColor Red
    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    exit 1
}
$title = Title-Of $trayHost
Write-Host ("INFO  {0}" -f $title) -ForegroundColor Cyan
Show-Result "config hotkeys registered" ($title -like "*quick=ok*" -and $title -like "*frozen=ok*" -and $title -like "*history=ok*") $title

# ---- 1. dynamic mode: mouse-up commits, no toolbar ----------------------
$pngBefore = Count-Files $dir "snip_*.png"

[void][S5.Native]::PostMessageW($trayHost, $WM_HOTKEY, [System.IntPtr]1, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 450
$ov = [S5.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
if ($ov -eq [System.IntPtr]::Zero) {
    Write-Host "FAIL  dynamic overlay did not open" -ForegroundColor Red
} else {
    Move-To $fx $fy
    [S5.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
    for ($i = 1; $i -le 8; $i++) {
        Move-To ($fx + [int](($tx - $fx) * $i / 8)) ($fy + [int](($ty - $fy) * $i / 8))
    }
    [S5.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)

    # watch for a toolbar for 200ms: in dynamic mode it must never appear
    $toolbarSeen = $false
    for ($i = 0; $i -lt 10; $i++) {
        if ([S5.Native]::FindWindowW("MySnipTool.Toolbar", [System.IntPtr]::Zero) -ne [System.IntPtr]::Zero) { $toolbarSeen = $true }
        Start-Sleep -Milliseconds 20
    }
    Show-Result "no toolbar in dynamic mode" (-not $toolbarSeen) $(if ($toolbarSeen) { "TOOLBAR APPEARED" } else { "never seen" })

    Start-Sleep -Milliseconds 800
    $ovGone = [S5.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero) -eq [System.IntPtr]::Zero
    Show-Result "overlay closes on mouse-up" $ovGone $(if ($ovGone) { "ok" } else { "STILL OPEN" })

    $pngAfter = Count-Files $dir "snip_*.png"
    Show-Result "png written on mouse-up" ($pngAfter -eq ($pngBefore + 1)) ("{0} -> {1}" -f $pngBefore, $pngAfter)

    $png = Get-ChildItem $dir -Filter "snip_*.png" | Sort-Object LastWriteTime -Descending | Select-Object -First 1
    $pw = 0; $ph = 0
    if ($png) {
        $img = [System.Drawing.Image]::FromFile($png.FullName)
        $pw = $img.Width; $ph = $img.Height
        $img.Dispose()
    }
    # This powershell is NOT dpi-aware, so Screen.Bounds / GetDpiForSystem are
    # both virtualised (they report 1280 wide and 96 dpi on a 2560@200% screen).
    # DESKTOPHORZRES is the one API that still reports real pixels, so derive
    # the scale from it. mouse_event ABSOLUTE normalises over physical pixels,
    # so the captured png is (0.62-0.28) * physical width.
    $hdc = [S5.Native]::GetDC([System.IntPtr]::Zero)
    $physW = [S5.Native]::GetDeviceCaps($hdc, 118)
    [void][S5.Native]::ReleaseDC([System.IntPtr]::Zero, $hdc)
    $dscale = if ($physW -gt 0) { $physW / $b.Width } else { 1 }
    $expectW = [int](($tx - $fx) * $dscale)
    $expectH = [int](($ty - $fy) * $dscale)
    $sizeOk = ([math]::Abs($pw - $expectW) -le 4) -and ([math]::Abs($ph - $expectH) -le 4)
    Show-Result "png matches the selection" $sizeOk ("{0}x{1} expect~{2}x{3} (scale {4})" -f $pw, $ph, $expectW, $expectH, $dscale)

    $clip = [System.Windows.Forms.Clipboard]::GetImage()
    $clipOk = $null -ne $clip
    $clipSize = if ($clip) { "{0}x{1}" -f $clip.Width, $clip.Height } else { "empty" }
    if ($clip) { $clip.Dispose() }
    Show-Result "clipboard filled on mouse-up" $clipOk $clipSize
}

# ---- 2. frozen mode still uses pending + toolbar -----------------------
[void][S5.Native]::PostMessageW($trayHost, $WM_HOTKEY, [System.IntPtr]2, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 1400
$ov = [S5.Native]::FindWindowW("MySnipTool.Overlay", [System.IntPtr]::Zero)
$frozenOk = $false
if ($ov -ne [System.IntPtr]::Zero) {
    Move-To $fx $fy
    [S5.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
    for ($i = 1; $i -le 8; $i++) {
        Move-To ($fx + [int](($tx - $fx) * $i / 8)) ($fy + [int](($ty - $fy) * $i / 8))
    }
    [S5.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 600

    $bar = [S5.Native]::FindWindowW("MySnipTool.Toolbar", [System.IntPtr]::Zero)
    $frozenOk = $bar -ne [System.IntPtr]::Zero
    Show-Result "frozen mode keeps the toolbar" $frozenOk $(if ($frozenOk) { "pending state ok" } else { "no toolbar" })

    $pngBeforeFrozen = Count-Files $dir "snip_*.png"
    [void][S5.Native]::PostMessageW($ov, $WM_KEYDOWN, [System.IntPtr]$VK_RETURN, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 1000
    $pngAfterFrozen = Count-Files $dir "snip_*.png"
    Show-Result "frozen confirm writes png" ($pngAfterFrozen -eq ($pngBeforeFrozen + 1)) ("{0} -> {1}" -f $pngBeforeFrozen, $pngAfterFrozen)
} else {
    Show-Result "frozen mode keeps the toolbar" $false "overlay did not open"
}

# ---- 3. settings window -------------------------------------------------
[void][S5.Native]::PostMessageW($trayHost, $WM_COMMAND, [System.IntPtr]$IDM_SETTINGS, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 900
$setWin = [S5.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero)
$setUp = $setWin -ne [System.IntPtr]::Zero
$edits = 0
$rectTxt = "none"
if ($setUp) {
    # the window is fully self-drawn now: the only native child left is the number EDIT
    $edits = Count-ChildClass $setWin "EDIT"
    $r = New-Object S5.Native+RECT
    [void][S5.Native]::GetWindowRect($setWin, [ref]$r)
    $rectTxt = "{0}x{1}" -f ($r.Right - $r.Left), ($r.Bottom - $r.Top)
}
Show-Result "settings window opens" ($setUp -and $edits -eq 1) ("hwnd=0x{0:X} edits={1} size={2}" -f $setWin.ToInt64(), $edits, $rectTxt)

# WS_EX_APPWINDOW (0x40000): without it the window has no taskbar button and no
# Alt+Tab entry, so clicking another app makes it look like it vanished
$ex = if ($setUp) { [S5.Native]::GetWindowLongPtrW($setWin, -20) } else { 0 }
Show-Result "settings reachable after blur" (($ex -band 0x40000) -ne 0) ("exstyle=0x{0:X}" -f $ex)

# click the blue save button (self-drawn, so this exercises hit-test + action)
if ($setUp) {
    $r = New-Object S5.Native+RECT
    [void][S5.Native]::GetWindowRect($setWin, [ref]$r)
    Move-To ($r.Left + 408) ($r.Top + 518)
    Start-Sleep -Milliseconds 150
    [S5.Native]::mouse_event($LDOWN, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 60
    [S5.Native]::mouse_event($LUP, 0, 0, 0, [System.IntPtr]::Zero)
    Start-Sleep -Milliseconds 1100
    $closed = [S5.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero) -eq [System.IntPtr]::Zero
    Show-Result "save button closes it" $closed $(if ($closed) { "ok" } else { "STILL OPEN" })
}

# one more open/close cycle: the window creates a font plus nine brushes,
# all of which must be released on WM_DESTROY
$gdiA = Get-Gdi $proc.Id
[void][S5.Native]::PostMessageW($trayHost, $WM_COMMAND, [System.IntPtr]$IDM_SETTINGS, [System.IntPtr]::Zero)
Start-Sleep -Milliseconds 800
$w2 = [S5.Native]::FindWindowW("MySnipTool.Settings", [System.IntPtr]::Zero)
if ($w2 -ne [System.IntPtr]::Zero) {
    [void][S5.Native]::PostMessageW($w2, $WM_CLOSE, [System.IntPtr]::Zero, [System.IntPtr]::Zero)
}
Start-Sleep -Milliseconds 800
$gdiB = Get-Gdi $proc.Id
Show-Result "settings leaks no GDI" ($gdiB -le $gdiA) ("before={0} after={1}" -f $gdiA, $gdiB)

$cfgText = if (Test-Path $cfgPath) { Get-Content $cfgPath -Raw } else { "" }
$keepCombo = $cfgText -match "history=Alt\+Shift\+V"
Show-Result "config survives save round-trip" $keepCombo $(($cfgText -split "`n" | Where-Object { $_ -match "^history=" }) -join "")
$keepRet = $cfgText -match "retention_hours=2"
Show-Result "retention survives save" $keepRet $(($cfgText -split "`n" | Where-Object { $_ -match "^retention_hours=" }) -join "")

# ---- 4. autostart registry ---------------------------------------------
$runValue = (Get-ItemProperty -Path $runKey -Name $valueName -ErrorAction SilentlyContinue).$valueName
$runOk = ($null -ne $runValue) -and ($runValue -like "*sniptool.exe*")
Show-Result "Run key points at the exe" $runOk $(if ($runValue) { $runValue } else { "MISSING" })

Write-Host ("INFO  gdi={0} ws={1} MB" -f (Get-Gdi $proc.Id), (Get-WsMb $proc.Id)) -ForegroundColor Cyan

# ---- restore ------------------------------------------------------------
# -KeepRunning leaves the app up for poking at, but the config and the Run
# value are still restored: they are the machine-level state, not the process.
if ($KeepRunning) {
    $script:proc = $null
    Restore-State
    Write-Host "sniptool left running (pid $($proc.Id))"
} else {
    Restore-State
}
