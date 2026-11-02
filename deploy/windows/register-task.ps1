# Register a weekly Windows scheduled task that runs vps-tidy (ASCII only: PowerShell 5.1 misreads UTF-8 without BOM).
# Usage (PowerShell, no admin needed):
#   .\register-task.ps1 -Exe "C:\Users\me\AppData\Local\vps-tidy\bin\vps-tidy.exe" [-Time "03:30"] [-Day Sunday] [-Apply]
# Without -Apply the task runs in dry-run mode (it only writes a report). Add -Apply to really delete.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [string]$Time = "03:30",
    [string]$Day = "Sunday",
    [switch]$Apply,
    [string]$TaskName = "vps-tidy"
)
if (-not (Test-Path -LiteralPath $Exe)) { throw "vps-tidy.exe not found: $Exe" }
$arg = if ($Apply) { "--apply" } else { "" }
$action = New-ScheduledTaskAction -Execute $Exe -Argument $arg
$trigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek $Day -At $Time
# StartWhenAvailable: if the PC was off at that time, run as soon as it is on. Low priority, no battery start.
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Hours 2) -AllowStartIfOnBatteries:$false -Priority 7
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Description "vps-tidy: tidy regenerable build leftovers by fixed rules" -Force | Out-Null
Write-Output ("Registered '{0}': every {1} at {2} ({3})" -f $TaskName, $Day, $Time, $(if ($Apply) { "APPLY" } else { "dry-run" }))
