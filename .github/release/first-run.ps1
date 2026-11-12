# A new person's first run on Windows, from what they took from the website: the download
# page, then the guide inside the archive, word for word, then a report written with the report
# form. Each line taken from a document is checked to be still in it before it runs.
# Changes this account's Path and home folder: run it only on a throwaway runner or container.
# -From is the folder holding what the website gave: the archive, SHA256SUMS,
# USER-AGREEMENT.md, download-page.md and reporting-a-problem.md.
param(
  [Parameter(Mandatory = $true)][string]$From,
  [Parameter(Mandatory = $true)][string]$Version
)
if ($env:GITHUB_ACTIONS -ne 'true') { Write-Host 'first-run.ps1 removes Opalro and its settings from this account and edits its Path: it runs only in CI'; exit 2 }
$ErrorActionPreference = 'Stop'
$script:current = ''
$zip = "opalro-$Version-x86_64-pc-windows-msvc.zip"
$Page = Join-Path $From download-page.md
$Form = Join-Path $From reporting-a-problem.md

function Step($name) { $script:current = $name; Write-Host ''; Write-Host "== $name" }
function Check($ok) { if (-not $ok) { throw 'the check failed' } }
# The line must be in the document.
function InDoc($doc, $line) {
  if (-not (Select-String -LiteralPath $doc -SimpleMatch -CaseSensitive -Pattern $line -Quiet)) {
    throw "no longer in ${doc}: $line"
  }
}
# A line from the guide, run as written.
function Run($line) {
  InDoc $script:guide $line
  Write-Host "> $line"
  $ErrorActionPreference = 'Continue'
  $global:LASTEXITCODE = 0
  $errors = $global:Error.Count
  Invoke-Expression $line | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "exited with $LASTEXITCODE" }
  if ($global:Error.Count -gt $errors) { throw $global:Error[0] }
}
# What a new terminal gets: the Path rebuilt from the registry.
function New-TerminalPath {
  $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
    [Environment]::GetEnvironmentVariable('Path', 'User')
}

try {
  Remove-Item Env:OPALRO_CONFIG_DIR, Env:OPALRO_USER_NAME, Env:OPALRO_USER_EMAIL -ErrorAction SilentlyContinue

  Step "the programs the guide's steps use"
  # A bare container may lack one: the log says so. Their work is done by this script anyway.
  foreach ($t in 'explorer', 'notepad') {
    if (-not (Get-Command $t -ErrorAction SilentlyContinue)) { Write-Host "::warning::This system has no $t, which the guide's steps use" }
  }

  Step 'the download page: the archive for this system, and SHA256SUMS, into the same folder'
  InDoc $Page "| ``$zip`` |"
  InDoc $Page 'Download the archive for your system, and `SHA256SUMS`, into the same folder.'
  $downloads = Join-Path $HOME Downloads
  New-Item -ItemType Directory -Force -Path $downloads | Out-Null
  Copy-Item -LiteralPath (Join-Path $From $zip), (Join-Path $From SHA256SUMS) -Destination $downloads
  Set-Location -LiteralPath $downloads
  # The guide is read where the download page says it is published; it is the archive's.
  $reading = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
  Expand-Archive -LiteralPath $zip -DestinationPath $reading
  $script:guide = Join-Path $reading docs\guide.md
  InDoc $Page "follow the user guide's section 2, Installing"

  Step 'the guide: check the download'
  $line = "(Get-FileHash .\$zip -Algorithm SHA256).Hash"
  InDoc $guide $line
  $hash = Invoke-Expression $line
  Write-Host $hash
  # Notepad's part: the line of SHA256SUMS that ends with the file's name.
  InDoc $guide 'compare it with the line that ends with the same file name (upper or lower case doesn''t matter)'
  $listed = @(Get-Content SHA256SUMS | Where-Object { $_.EndsWith(" $zip") })
  Check ($listed.Count -eq 1)
  Write-Host $listed[0]
  Check ($hash -eq $listed[0].Split(' ')[0])

  Step 'the guide: unpack with Extract All, and move opalro.exe to a folder of your own'
  InDoc $guide 'unpack the `.zip` (right-click it, then "Extract All")'
  # Extract All proposes a folder named after the archive, beside it.
  $unpacked = Join-Path $downloads ([IO.Path]::GetFileNameWithoutExtension($zip))
  Expand-Archive -LiteralPath $zip -DestinationPath $unpacked
  InDoc $Page 'Read the user agreement, `USER-AGREEMENT.md`.'
  Check (Test-Path -LiteralPath (Join-Path $unpacked USER-AGREEMENT.md))
  # The agreement offered beside the downloads is the archive's, byte for byte.
  InDoc $Page 'offered here beside the downloads. It is also inside every archive.'
  $offered = (Get-FileHash -LiteralPath (Join-Path $From USER-AGREEMENT.md) -Algorithm SHA256).Hash
  $inside = (Get-FileHash -LiteralPath (Join-Path $unpacked USER-AGREEMENT.md) -Algorithm SHA256).Hash
  Write-Host "USER-AGREEMENT.md beside the downloads: $offered, in the archive: $inside"
  Check ($offered -eq $inside)
  InDoc $guide 'Move `opalro.exe` to a folder of your own, such as `C:\Users\<you>\bin` (make the folder first if it isn''t there).'
  $bin = Join-Path $HOME bin
  New-Item -ItemType Directory -Path $bin | Out-Null
  Move-Item -LiteralPath (Join-Path $unpacked opalro.exe) -Destination $bin

  Step "the guide: add that folder to the account's Path"
  InDoc $guide 'Open Start, search for "environment variables", open "Edit environment variables for your account", choose `Path`, then "Edit", "New", and add that folder. Open a new terminal.'
  # Not found before, so the folder added is what puts it there.
  New-TerminalPath
  Check (-not (Get-Command opalro -ErrorAction SilentlyContinue))
  # The dialog's work, in the registry.
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $userPath = (@($userPath -split ';' | Where-Object { $_ }) + $bin) -join ';'
  [Environment]::SetEnvironmentVariable('Path', $userPath, 'User')

  Step 'the guide: in a new terminal, opalro --version'
  New-TerminalPath
  $found = (Get-Command opalro).Source
  Write-Host "found: $found"
  Check ($found -eq (Join-Path $bin opalro.exe))
  InDoc $guide 'opalro --version'
  $said = (powershell -NoProfile -Command 'opalro --version') -join "`n"
  Write-Host $said
  Check ($said -ceq "opalro $Version")

  Step 'the guide: tell Opalro who you are'
  Run 'mkdir -Force $env:APPDATA\opalro\config'
  InDoc $guide 'notepad $env:APPDATA\opalro\config\config.toml'
  # Notepad's work: the guide's three lines, saved as UTF-8 with Windows line ends.
  $lines = '[user]', 'name = "Ana Lima"', 'email = "ana@example.com"'
  foreach ($l in $lines) { InDoc $guide $l }
  [IO.File]::WriteAllText("$env:APPDATA\opalro\config\config.toml", ($lines -join "`r`n") + "`r`n",
    (New-Object Text.UTF8Encoding $false))

  Step 'the guide: the first five minutes, in a new folder'
  $practice = Join-Path $HOME practice
  New-Item -ItemType Directory -Path $practice | Out-Null
  Set-Location -LiteralPath $practice
  # The ten lines below are every line of the section's sh blocks; one added there fails here.
  $in3 = $false; $inSh = $false; $count = 0
  foreach ($l in Get-Content -LiteralPath $guide -Encoding UTF8) {
    if ($l -match '^## 3\.') { $in3 = $true } elseif ($l -match '^## 4\.') { $in3 = $false }
    if (-not $in3) { continue }
    if ($l.StartsWith('```')) { $inSh = (-not $inSh) -and ($l -ceq '```sh') } elseif ($inSh) { $count++ }
  }
  Check ($count -eq 10)
  Run 'opalro start --preset code'
  Run 'echo "hello" > notes.txt'
  Run 'opalro save "First notes"'
  Run 'opalro status'
  Run 'echo "hello again" > notes.txt'
  Run 'opalro status'
  Run 'opalro changes'
  Run 'opalro save "More notes"'
  Run 'opalro history'
  Run 'opalro undo'
  Check (((Get-Content notes.txt) -join "`n") -ceq 'hello again')

  Step 'a report, written with the report form'
  InDoc $Page '"Reporting a problem", published on this page, says what to send.'
  $os = Get-CimInstance Win32_OperatingSystem
  $saidVersion = (opalro --version | Out-String).Trim(); Check ($LASTEXITCODE -eq 0)
  $saidStatus = (opalro status | Out-String).TrimEnd(); Check ($LASTEXITCODE -eq 0)
  $saidVerify = (opalro verify | Out-String).TrimEnd(); Check ($LASTEXITCODE -eq 0)
  $saidJson = (opalro status --json | Out-String).Trim(); Check ($LASTEXITCODE -eq 0)
  if (-not $saidJson.Contains('"opalro_json":1')) { throw 'opalro status --json lacks "opalro_json":1' }
  $script:report = @()
  # A part of the report, under a label of the form's "What to send" list.
  function Part($label, $text) {
    InDoc $Form "- **$label**"
    $script:report += ($label.TrimEnd([char[]]',:') + ':'), $text, ''
  }
  Part 'The version:' $saidVersion
  Part 'Your system and its version:' "$($os.Caption) $($os.Version)"
  Part 'What you typed:' "opalro status`r`n(in the guide's practice project, just after opalro undo)"
  Part 'What it printed:' $saidStatus
  Part 'What `opalro verify` says,' $saidVerify
  Part 'If it helps, the same command with `--json`,' "> opalro status --json`r`n$saidJson"
  InDoc $Form '- What you expected to happen instead.'
  $script:report += 'What you expected to happen instead:',
    'Nothing else: this report is a rehearsal. opalro status names notes.txt as changed after opalro undo, as the guide''s section 3 says.'
  # Every bold label of the list is one above; one added there fails here.
  $inList = $false; $bold = 0
  foreach ($l in Get-Content -LiteralPath $Form -Encoding UTF8) {
    if ($l.StartsWith('## ')) { $inList = $l -ceq '## What to send' } elseif ($inList -and $l.StartsWith('- **')) { $bold++ }
  }
  if ($bold -ne 6) { throw "the form's ""What to send"" list has $bold bold labels, not the 6 this report fills" }
  $reportFile = Join-Path $downloads report.txt
  [IO.File]::WriteAllLines($reportFile, [string[]]$script:report)
  Get-Content -LiteralPath $reportFile -Encoding UTF8 | Write-Host

  Step 'the guide: removing Opalro from a computer'
  Set-Location -LiteralPath $HOME
  Run 'Remove-Item $HOME\bin\opalro.exe'
  Run 'Remove-Item -Recurse $env:APPDATA\opalro'
  Check (-not (Test-Path -LiteralPath "$env:APPDATA\opalro"))
  InDoc $guide 'Then take the folder off your PATH: open Start, search for "environment variables", open "Edit environment variables for your account", choose `Path`, then "Edit", select the folder and choose "Delete". Leave it if other programs live in it.'
  Check (@(Get-ChildItem -LiteralPath $bin -Force).Count -eq 0)
  # The dialog's work, in the registry.
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $userPath = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $bin }) -join ';'
  [Environment]::SetEnvironmentVariable('Path', $userPath, 'User')

  Step 'in a new terminal, opalro is no longer found'
  New-TerminalPath
  Write-Host "Path: $env:Path"
  Check (-not (@($env:Path -split ';') -contains $bin))
  Check (-not (Get-Command opalro -ErrorAction SilentlyContinue))

  Remove-Item -LiteralPath $reading, $practice, $unpacked -Recurse -Force
  Write-Host ''
  Write-Host 'first-run passed: x86_64-pc-windows-msvc'
} catch {
  Write-Host "first-run FAILED at: $script:current ($_)"
  exit 1
}
