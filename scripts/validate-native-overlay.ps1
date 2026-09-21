param([int]$ProcessId, [Int64]$WindowHandle)

# Read-only Windows validation: no input injection, window moves, or state changes.
if (-not ("OverlayNative" -as [type])) {
  Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class OverlayNative {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
  [DllImport("user32.dll", SetLastError=true)] public static extern int GetWindowLong(IntPtr hWnd, int index);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hWnd);
}
'@
}
$process = if ($ProcessId) { Get-Process -Id $ProcessId -ErrorAction Stop } else { Get-Process llm-quota-overlay -ErrorAction Stop | Select-Object -First 1 }
$handle = if ($WindowHandle) { [IntPtr]$WindowHandle } else { $process.MainWindowHandle }
if ($handle -eq [IntPtr]::Zero) { throw "No native overlay window is available for process $($process.Id)." }
$rect = New-Object OverlayNative+RECT
[void][OverlayNative]::GetWindowRect($handle, [ref]$rect)
$style = [OverlayNative]::GetWindowLong($handle, -16)
$exStyle = [OverlayNative]::GetWindowLong($handle, -20)
[pscustomobject]@{
  ProcessId = $process.Id
  Visible = [OverlayNative]::IsWindowVisible($handle)
  X = $rect.Left; Y = $rect.Top
  Width = $rect.Right - $rect.Left; Height = $rect.Bottom - $rect.Top
  FramelessStyleBitAdvisory = (($style -band 0x00C00000) -eq 0)
  ToolWindowStyleBitAdvisory = (($exStyle -band 0x00000080) -ne 0)
  AlwaysOnTop = (($exStyle -band 0x00000008) -ne 0)
  StyleHex = ('0x{0:X8}' -f $style)
  ExStyleHex = ('0x{0:X8}' -f $exStyle)
}
