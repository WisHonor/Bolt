# Takes a release program through everyday commands in a fresh place, with the project and
# the settings in folders whose names hold spaces and letters outside ASCII.
# Runs on Windows PowerShell 5.1, as Server Core has no other.
# Stops at the first step that fails, and names it.
param(
  [Parameter(Mandatory = $true)][string]$Opalro,
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$GitRepo
)
$ErrorActionPreference = 'Stop'
$script:current = ''

function Step($name) { $script:current = $name; Write-Host ''; Write-Host "== $name" }
function Check($ok) { if (-not $ok) { throw 'the check failed' } }
function Run-Opalro {
  # Opalro's notes on stderr are not errors; its exit code says.
  $ErrorActionPreference = 'Continue'
  & $Opalro @args
  if ($LASTEXITCODE -ne 0) { throw "opal $args exited with $LASTEXITCODE" }
}

# Built from code points: Windows PowerShell reads a script without a byte-order mark as ANSI.
$e = [char]0xE9
$nihon = [string][char]0x65E5 + [char]0x672C

try {
  $work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
  $root = Join-Path $work "Projets d'${e}t${e}\$nihon"
  $env:OPAL_CONFIG_DIR = Join-Path $work "r${e}glages de opal"
  Remove-Item Env:OPAL_USER_NAME, Env:OPAL_USER_EMAIL -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Path $root, $env:OPAL_CONFIG_DIR | Out-Null
  # With a byte-order mark, as Notepad may write it.
  Set-Content -LiteralPath (Join-Path $env:OPAL_CONFIG_DIR config.toml) -Encoding UTF8 -Value @(
    '[user]', 'name = "Clean Machine"', 'email = "clean@example.invalid"')

  Step "opal --version prints opal $Version"
  $said = (Run-Opalro --version) -join "`n"
  Write-Host $said
  Check ($said -ceq "opal $Version")

  foreach ($preset in 'code', 'game', 'custom') {
    Step "start --preset $preset in a new folder"
    New-Item -ItemType Directory -Path (Join-Path $root $preset) | Out-Null
    Set-Location -LiteralPath (Join-Path $root $preset)
    Run-Opalro start --preset $preset
  }

  Set-Location -LiteralPath (Join-Path $root code)
  Step 'a file, then save First'
  Set-Content notes.txt first
  Run-Opalro save First

  Step 'a change, and status names it'
  Set-Content notes.txt second
  $status = Run-Opalro status | Out-String
  Write-Host $status
  Check ($status -match 'notes\.txt')

  Step 'save Second'
  Run-Opalro save Second

  Step 'goto the first save, by the change ID history --json gives'
  # Newest first: Second, then First.
  $first = (Run-Opalro history --json | Out-String | ConvertFrom-Json).saves[1].change_id
  Write-Host "First is $first"
  Check ($first)
  Run-Opalro goto $first

  Step 'the file holds the first contents'
  Check (((Get-Content notes.txt) -join "`n") -ceq 'first')

  Step 'undo: the second contents are back'
  Run-Opalro undo
  Check (((Get-Content notes.txt) -join "`n") -ceq 'second')

  Step 'redo: the first contents again'
  Run-Opalro redo
  Check (((Get-Content notes.txt) -join "`n") -ceq 'first')

  Step 'a change, put aside with shelf: the folder is clean again'
  Set-Content notes.txt third
  Run-Opalro shelf Third
  Check (((Get-Content notes.txt) -join "`n") -ceq 'first')
  $now = Run-Opalro status --json | Out-String
  Write-Host $now
  Check (@(($now | ConvertFrom-Json).changes).Count -eq 0)

  Step 'verify'
  Run-Opalro verify

  Step 'import a Git repository into a copy of it'
  $imported = Join-Path $root imported
  Copy-Item -LiteralPath $GitRepo -Destination $imported -Recurse -Force
  Set-Location -LiteralPath $imported
  Run-Opalro import

  Step 'history of the import'
  $history = Run-Opalro history | Out-String
  Write-Host $history
  Check ($history -match 'Add notes')

  Step 'verify the import'
  Run-Opalro verify

  Set-Location -LiteralPath ([IO.Path]::GetTempPath())
  Remove-Item -LiteralPath $work -Recurse -Force
  Write-Host ''
  Write-Host "clean-machine passed: $said"
} catch {
  Write-Host "clean-machine FAILED at: $script:current ($_)"
  exit 1
}
