param([Parameter(Mandatory=$true)][int]$ProcessId, [int]$Seconds = 20)

# Read-only sampling: no window activation, input, configuration, or process mutation.
if ($Seconds -lt 1) { throw 'Seconds must be at least 1.' }
function Get-OverlayProcesses([int]$RootId) {
  $all = @(Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name)
  $ids = [System.Collections.Generic.HashSet[int]]::new(); [void]$ids.Add($RootId)
  do { $added = $false; foreach ($item in $all) { if ($ids.Contains([int]$item.ParentProcessId) -and $ids.Add([int]$item.ProcessId)) { $added = $true } } } while ($added)
  $ids | ForEach-Object { Get-Process -Id $_ -ErrorAction SilentlyContinue } | Where-Object { $_ }
}
function Get-Sample([int]$RootId) {
  $items = @(Get-OverlayProcesses $RootId)
  [pscustomobject]@{ Cpu = ($items | Measure-Object CPU -Sum).Sum; Working = ($items | Measure-Object WorkingSet64 -Sum).Sum; Private = ($items | Measure-Object PrivateMemorySize64 -Sum).Sum; Pids = @($items.Id) }
}
$start = Get-Sample $ProcessId
Start-Sleep -Seconds $Seconds
$end = Get-Sample $ProcessId
$cores = [Environment]::ProcessorCount
$cpuSeconds = [Math]::Max(0, [double]$end.Cpu - [double]$start.Cpu)
$corePercent = 100 * $cpuSeconds / $Seconds
[pscustomobject]@{
  ProcessIds = ($end.Pids -join ',')
  SampleSeconds = $Seconds
  CoreEquivalentCpuPercent = [Math]::Round($corePercent, 3)
  MachineCpuPercent = [Math]::Round($corePercent / $cores, 3)
  WorkingSetMB = [Math]::Round(([double]$end.Working / 1MB), 2)
  PrivateBytesMB = [Math]::Round(([double]$end.Private / 1MB), 2)
}
