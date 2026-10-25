param([string]$Exe, [string]$Steps, [string]$OutDir, [string]$AppData)
# Dev check of the settings UI. Starts the app without focus (SW_SHOWNOACTIVATE), with APPDATA at a
# scratch dir so the real settings stay untouched, and runs steps with posted messages:
#   powershell -File scripts/drive-settings.ps1 -Exe target/release/brushkey-settings.exe -Steps "shot a; click 291 68; shot b" -OutDir <dir> -AppData <scratch dir>
# Steps, separated by ';': wait <ms> | move <x> <y> | click <x> <y> | wheel <x> <y> <notches> | key <vk hex> | shot <name>
# Coordinates are client DIPs (the screenshots include the frame, about 8 x 31 px more); the script scales them
# by the window's DPI. Posted moves cannot show hover: the real cursor is elsewhere, so WM_MOUSELEAVE follows at once.
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class D {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct SI {
    public int cb; public string r, desk, title; public int x, y, w, h, xc, yc, fill, flags; public short show, r2; public IntPtr r3, i, o, e; }
  [StructLayout(LayoutKind.Sequential)] public struct PI { public IntPtr hp, ht; public int pid, tid; }
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern bool CreateProcessW(string app, string cmd, IntPtr pa, IntPtr ta, bool inherit, int flags, IntPtr env, string dir, ref SI si, out PI pi);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint f);
  [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
  [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
}
"@
$env:APPDATA = $AppData
$si = New-Object D+SI
$si.cb = [Runtime.InteropServices.Marshal]::SizeOf($si); $si.flags = 1; $si.show = 4
$pi = New-Object D+PI
if (-not [D]::CreateProcessW($Exe, "`"$Exe`"", [IntPtr]::Zero, [IntPtr]::Zero, $false, 0, [IntPtr]::Zero, [NullString]::Value, [ref]$si, [ref]$pi)) { "CreateProcess failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"; exit 1 }
$p = Get-Process -Id $pi.pid
$sw = [Diagnostics.Stopwatch]::StartNew()
while ($p.MainWindowHandle -eq 0 -and $sw.ElapsedMilliseconds -lt 15000) { Start-Sleep -Milliseconds 50; $p.Refresh() }
$h = $p.MainWindowHandle
Start-Sleep -Milliseconds 400
$s = [D]::GetDpiForWindow($h) / 96.0
"pid $($pi.pid) scale $s foreground-is-ours $([D]::GetForegroundWindow() -eq $h)"
function MakeLParam($x, $y) { [IntPtr](([int]$y -shl 16) -bor ([int]$x -band 0xffff)) }
function Px($v) { [int]([double]$v * $s) }
function Post($m, $w, $l) { [void][D]::PostMessage($h, $m, [IntPtr]$w, $l) }
foreach ($step in $Steps.Split(';')) {
  $a = $step.Trim().Split(' ')
  switch ($a[0]) {
    'wait' { Start-Sleep -Milliseconds ([int]$a[1]) }
    'move' { Post 0x200 0 (MakeLParam (Px $a[1]) (Px $a[2])); Start-Sleep -Milliseconds 150 }
    'click' { $l = MakeLParam (Px $a[1]) (Px $a[2]); Post 0x200 0 $l; Start-Sleep -Milliseconds 60; Post 0x201 1 $l; Start-Sleep -Milliseconds 40; Post 0x202 0 $l; Start-Sleep -Milliseconds 300 }
    'wheel' {
      $pt = New-Object D+POINT; $pt.X = Px $a[1]; $pt.Y = Px $a[2]; [void][D]::ClientToScreen($h, [ref]$pt)
      Post 0x20A ([int]$a[3] * 120 -shl 16) (MakeLParam $pt.X $pt.Y); Start-Sleep -Milliseconds 200
    }
    'key' { $vk = [Convert]::ToInt32($a[1], 16); Post 0x100 $vk 0; Post 0x101 $vk 0; Start-Sleep -Milliseconds 300 }
    'shot' {
      $r = New-Object D+RECT
      [void][D]::GetWindowRect($h, [ref]$r)
      $bmp = New-Object Drawing.Bitmap ($r.R - $r.L), ($r.B - $r.T)
      $g = [Drawing.Graphics]::FromImage($bmp); $dc = $g.GetHdc()
      [void][D]::PrintWindow($h, $dc, 2); $g.ReleaseHdc($dc)
      $bmp.Save((Join-Path $OutDir "$($a[1]).png"))
    }
  }
}
if (-not $p.HasExited) { Stop-Process -Id $pi.pid -Force }
