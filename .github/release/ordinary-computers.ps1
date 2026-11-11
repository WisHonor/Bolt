# Tries a release program where ordinary computers keep projects: a FAT32 and an exFAT drive
# (a VHD each), the top of such a drive, a network share, and a file another program holds.
# Prints what Bolt says; a refusal is written down, not a failure. Fails only if a try can't
# be set up.
param([Parameter(Mandatory = $true)][string]$Bolt)
$ErrorActionPreference = 'Stop'

$work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
$env:BOLT_CONFIG_DIR = Join-Path $work config
$config = Join-Path $env:BOLT_CONFIG_DIR config.toml
$user = @('[user]', 'name = "Ana Lima"', 'email = "ana@example.com"')
Remove-Item Env:BOLT_USER_NAME, Env:BOLT_USER_EMAIL -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $env:BOLT_CONFIG_DIR | Out-Null

# Runs bolt, prints all it said and its exit code, and hands back what it said.
function Try-Bolt {
  $ErrorActionPreference = 'Continue'
  Write-Host "> bolt $args"
  $said = @(& $Bolt @args 2>&1 | ForEach-Object { "$_" })
  $said | ForEach-Object { Write-Host "  $_" }
  Write-Host "  (exit $LASTEXITCODE)"
  $said
}

# Start, two saves, a goto back to the first, verify.
function Use-Project($dir) {
  Set-Location -LiteralPath $dir
  $said = @(Try-Bolt start --preset custom)
  Set-Content a.txt one
  $said += Try-Bolt save First
  Set-Content a.txt two
  $null = Try-Bolt save Second
  $first = $null
  try { $first = ((Try-Bolt history --json) -join "`n" | ConvertFrom-Json).saves[1].change_id } catch {}
  if ($first) { $null = Try-Bolt goto $first; Write-Host "  a.txt now holds: $(Get-Content a.txt)" }
  $null = Try-Bolt verify
  $said
}

# The case, then, if Bolt offered a safe-folders line, the case again with that line added.
function Try-Case($name, $dir) {
  Write-Host ''
  Write-Host "::group::$name ($dir)"
  Set-Content -LiteralPath $config -Encoding UTF8 -Value $user
  if (-not (Test-Path -LiteralPath $dir)) { New-Item -ItemType Directory -Path $dir | Out-Null }
  $said = Use-Project $dir
  $line = $said | Where-Object { $_ -match '^\s*safe-folders = ' } | Select-Object -First 1
  if ($line) {
    $line = $line.Trim()
    Write-Host "-- again, with the hint's line at the top of the settings: $line"
    Set-Content -LiteralPath $config -Encoding UTF8 -Value (@($line) + $user)
    $null = Use-Project $dir
  } else {
    Write-Host '-- Bolt offered no safe-folders line'
  }
  Set-Location -LiteralPath $work
  Write-Host '::endgroup::'
}

function New-Drive($fs, $letter) {
  $vhd = Join-Path $work "$fs.vhdx"
  $script = Join-Path $work "$fs.txt"
  Set-Content -LiteralPath $script -Encoding ASCII -Value @(
    "create vdisk file=`"$vhd`" maximum=512 type=expandable",
    "select vdisk file=`"$vhd`"",
    'attach vdisk',
    'create partition primary',
    "format fs=$fs quick label=BOLT$fs",
    "assign letter=$letter")
  diskpart /s $script | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "diskpart couldn't make the $fs drive" }
  Get-Volume -DriveLetter $letter | Format-List DriveLetter, FileSystem, Size | Out-Host
}

Write-Host "New-VHD here: $([bool](Get-Command New-VHD -ErrorAction SilentlyContinue)); the drives are made with diskpart"
Write-Host "Running as: $([Security.Principal.WindowsIdentity]::GetCurrent().Name)"

foreach ($d in @(@('FAT32', 'R'), @('exFAT', 'S'))) {
  $fs, $letter = $d
  New-Drive $fs $letter
  Try-Case "$fs, a folder" "${letter}:\proj"
  Try-Case "$fs, the top of the drive" "${letter}:\"
}

# A share of a folder on this computer, mapped to a drive letter.
$shared = Join-Path $work shared
New-Item -ItemType Directory -Path $shared | Out-Null
New-SmbShare -Name boltshare -Path $shared -FullAccess Everyone | Out-Host
net use T: \\localhost\boltshare | Out-Host
if ($LASTEXITCODE -ne 0) { throw "couldn't map the share" }
Try-Case 'a network share, a folder' 'T:\proj'
Try-Case 'a network share, its top' 'T:\'

# A file another program holds with no sharing, as a virus scanner may, during a save.
$held = Join-Path $work held
New-Item -ItemType Directory -Path $held | Out-Null
Set-Content -LiteralPath $config -Encoding UTF8 -Value $user
Write-Host ''
Write-Host "::group::a file held open during a save ($held)"
Set-Location -LiteralPath $held
$null = Try-Bolt start --preset custom
Set-Content a.txt one
$h = [IO.File]::Open((Join-Path $held a.txt), 'Open', 'Read', 'None')
$null = Try-Bolt save First
$h.Close()
Write-Host '-- let go'
$null = Try-Bolt save First
Write-Host '::endgroup::'
Set-Location -LiteralPath $work
Write-Host ''
Write-Host 'ordinary-computers: every try ran'
