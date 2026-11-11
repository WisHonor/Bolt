# Takes a release program through everyday commands in a fresh place, with the project and
# the settings in folders whose names hold spaces and letters outside ASCII.
# Runs on Windows PowerShell 5.1, as Server Core has no other.
# Stops at the first step that fails, and names it.
param(
  [Parameter(Mandatory = $true)][string]$Bolt,
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$GitRepo
)
$ErrorActionPreference = 'Stop'
$script:current = ''

function Step($name) { $script:current = $name; Write-Host ''; Write-Host "== $name" }
function Check($ok) { if (-not $ok) { throw 'the check failed' } }
function Run-Bolt {
  # Bolt's notes on stderr are not errors; its exit code says.
  $ErrorActionPreference = 'Continue'
  & $Bolt @args
  if ($LASTEXITCODE -ne 0) { throw "bolt $args exited with $LASTEXITCODE" }
}

# Built from code points: Windows PowerShell reads a script without a byte-order mark as ANSI.
$e = [char]0xE9
$nihon = [string][char]0x65E5 + [char]0x672C

try {
  $work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
  $root = Join-Path $work "Projets d'${e}t${e}\$nihon"
  $env:BOLT_CONFIG_DIR = Join-Path $work "r${e}glages de bolt"
  Remove-Item Env:BOLT_USER_NAME, Env:BOLT_USER_EMAIL -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Path $root, $env:BOLT_CONFIG_DIR | Out-Null
  # With a byte-order mark, as Notepad may write it.
  Set-Content -LiteralPath (Join-Path $env:BOLT_CONFIG_DIR config.toml) -Encoding UTF8 -Value @(
    '[user]', 'name = "Clean Machine"', 'email = "clean@example.invalid"')

  Step "bolt --version prints bolt $Version"
  $said = (Run-Bolt --version) -join "`n"
  Write-Host $said
  Check ($said -ceq "bolt $Version")

  foreach ($preset in 'code', 'game', 'custom') {
    Step "start --preset $preset in a new folder"
    New-Item -ItemType Directory -Path (Join-Path $root $preset) | Out-Null
    Set-Location -LiteralPath (Join-Path $root $preset)
    Run-Bolt start --preset $preset
  }

  Set-Location -LiteralPath (Join-Path $root code)
  Step 'a file, then save First'
  Set-Content notes.txt first
  Run-Bolt save First

  Step 'a change, and status names it'
  Set-Content notes.txt second
  $status = Run-Bolt status | Out-String
  Write-Host $status
  Check ($status -match 'notes\.txt')

  Step 'save Second'
  Run-Bolt save Second

  Step 'goto the first save, by the change ID history --json gives'
  # Newest first: Second, then First.
  $first = (Run-Bolt history --json | Out-String | ConvertFrom-Json).saves[1].change_id
  Write-Host "First is $first"
  Check ($first)
  Run-Bolt goto $first

  Step 'the file holds the first contents'
  Check (((Get-Content notes.txt) -join "`n") -ceq 'first')

  Step 'undo: the second contents are back'
  Run-Bolt undo
  Check (((Get-Content notes.txt) -join "`n") -ceq 'second')

  Step 'redo: the first contents again'
  Run-Bolt redo
  Check (((Get-Content notes.txt) -join "`n") -ceq 'first')

  Step 'a change, put aside with shelf: the folder is clean again'
  Set-Content notes.txt third
  Run-Bolt shelf Third
  Check (((Get-Content notes.txt) -join "`n") -ceq 'first')
  $now = Run-Bolt status --json | Out-String
  Write-Host $now
  Check (@(($now | ConvertFrom-Json).changes).Count -eq 0)

  Step 'verify'
  Run-Bolt verify

  Step 'import a Git repository into a copy of it'
  $imported = Join-Path $root imported
  Copy-Item -LiteralPath $GitRepo -Destination $imported -Recurse -Force
  Set-Location -LiteralPath $imported
  Run-Bolt import

  Step 'history of the import'
  $history = Run-Bolt history | Out-String
  Write-Host $history
  Check ($history -match 'Add notes')

  Step 'verify the import'
  Run-Bolt verify

  Set-Location -LiteralPath ([IO.Path]::GetTempPath())
  Remove-Item -LiteralPath $work -Recurse -Force
  Write-Host ''
  Write-Host "clean-machine passed: $said"
} catch {
  Write-Host "clean-machine FAILED at: $script:current ($_)"
  exit 1
}
