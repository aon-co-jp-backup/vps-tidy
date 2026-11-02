# Register a weekly Windows scheduled task that runs vps-tidy (ASCII only: PowerShell 5.1 misreads UTF-8 without BOM).
# Usage (PowerShell, no admin needed):
#   .\register-task.ps1 -Exe "F:\vps-tidy\bin\vps-tidy.exe" -Config "F:\vps-tidy\bin\tidy.toml" [-Time "03:30"] [-Day Sunday] [-Apply]
# Without -Apply the task runs in dry-run mode (it only writes a report). Add -Apply to really delete.
param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(Mandatory = $true)][string]$Config,
    [string]$Time = "03:30",
    [string]$Day = "Sunday",
    [switch]$Apply,
    [string]$TaskName = "vps-tidy"
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path -LiteralPath $Exe)) { throw "vps-tidy.exe not found: $Exe" }

# New-ScheduledTaskAction rejects an empty -Argument, so pass it only when there is one.
# Always pass an explicit config path and a working directory: under the scheduler %LOCALAPPDATA% may point to another
# profile, and an exe kept under a sandboxed/virtualised profile path may be invisible to it (0x80070002).
# Keep the exe and the config on a plain path such as F:\vps-tidy\bin.
$argLine = "--config `"$Config`""
if ($Apply) { $argLine += " --apply" }
$action = New-ScheduledTaskAction -Execute $Exe -Argument $argLine -WorkingDirectory (Split-Path -Parent $Exe)
$trigger = New-ScheduledTaskTrigger -Weekly -DaysOfWeek $Day -At $Time
# Run as the current user with its full name (DOMAIN-slash-user). A short name can make the scheduler fail to resolve
# the account and return 0x80070002 (file not found) even though the exe exists.
$me = [System.Security.Principal.WindowsIdentity]::GetCurrent().Name
$principal = New-ScheduledTaskPrincipal -UserId $me -LogonType Interactive -RunLevel Limited
# StartWhenAvailable: if the PC was off at that time, run as soon as it is on. Low priority, no battery start.
$settings = New-ScheduledTaskSettingsSet -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Hours 2) -AllowStartIfOnBatteries:$false -Priority 7
Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings -Principal $principal -Description "vps-tidy: tidy regenerable build leftovers by fixed rules" -Force | Out-Null

# Never claim success unless the task really exists now.
$t = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
if (-not $t) { throw "Registration failed: task '$TaskName' was not created." }
$mode = if ($Apply) { "APPLY" } else { "dry-run" }
Write-Output ("Registered '{0}': every {1} at {2} ({3})" -f $TaskName, $Day, $Time, $mode)
