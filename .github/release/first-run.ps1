# A new person's first run on Windows: the download page, then the guide inside the archive,
# word for word. Each line taken from a document is checked to be still in it before it runs.
# Changes this account's Path and home folder: run it only on a throwaway runner.
param(
  [Parameter(Mandatory = $true)][string]$From,
  [Parameter(Mandatory = $true)][string]$Version,
  [Parameter(Mandatory = $true)][string]$Page
)
$ErrorActionPreference = 'Stop'
$script:current = ''
$zip = "bolt-$Version-x86_64-pc-windows-msvc.zip"

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
  Invoke-Expression $line | Out-Host
  if ($LASTEXITCODE -ne 0) { throw "exited with $LASTEXITCODE" }
}
# What a new terminal gets: the Path rebuilt from the registry.
function New-TerminalPath {
  $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' +
    [Environment]::GetEnvironmentVariable('Path', 'User')
}

try {
  Remove-Item Env:BOLT_CONFIG_DIR, Env:BOLT_USER_NAME, Env:BOLT_USER_EMAIL -ErrorAction SilentlyContinue

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

  Step 'the guide: unpack with Extract All, and move bolt.exe to a folder of your own'
  InDoc $guide 'unpack the `.zip` (right-click it, then "Extract All")'
  # Extract All proposes a folder named after the archive, beside it.
  $unpacked = Join-Path $downloads ([IO.Path]::GetFileNameWithoutExtension($zip))
  Expand-Archive -LiteralPath $zip -DestinationPath $unpacked
  InDoc $Page 'Read the user agreement, `USER-AGREEMENT.md`.'
  Check (Test-Path -LiteralPath (Join-Path $unpacked USER-AGREEMENT.md))
  InDoc $guide 'Move `bolt.exe` to a folder of your own, such as `C:\Users\<you>\bin` (make the folder first if it isn''t there).'
  $bin = Join-Path $HOME bin
  New-Item -ItemType Directory -Path $bin | Out-Null
  Move-Item -LiteralPath (Join-Path $unpacked bolt.exe) -Destination $bin

  Step "the guide: add that folder to the account's Path"
  InDoc $guide 'Open Start, search for "environment variables", open "Edit environment variables for your account", choose `Path`, then "Edit", "New", and add that folder. Open a new terminal.'
  # The dialog's work, in the registry.
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $userPath = (@($userPath -split ';' | Where-Object { $_ }) + $bin) -join ';'
  [Environment]::SetEnvironmentVariable('Path', $userPath, 'User')

  Step 'the guide: in a new terminal, bolt --version'
  New-TerminalPath
  $found = (Get-Command bolt).Source
  Write-Host "found: $found"
  Check ($found -eq (Join-Path $bin bolt.exe))
  InDoc $guide 'bolt --version'
  $said = powershell -NoProfile -Command 'bolt --version'
  Write-Host $said
  Check ($said -ceq "bolt $Version")

  Step 'the guide: tell Bolt who you are'
  Run 'mkdir -Force $env:APPDATA\bolt\config'
  InDoc $guide 'notepad $env:APPDATA\bolt\config\config.toml'
  # Notepad's work: the guide's three lines, saved as UTF-8 with Windows line ends.
  $lines = '[user]', 'name = "Ana Lima"', 'email = "ana@example.com"'
  foreach ($l in $lines) { InDoc $guide $l }
  [IO.File]::WriteAllText("$env:APPDATA\bolt\config\config.toml", ($lines -join "`r`n") + "`r`n",
    (New-Object Text.UTF8Encoding $false))

  Step 'the guide: the first five minutes, in a new folder'
  $practice = Join-Path $HOME practice
  New-Item -ItemType Directory -Path $practice | Out-Null
  Set-Location -LiteralPath $practice
  Run 'bolt start --preset code'
  Run 'echo "hello" > notes.txt'
  Run 'bolt save "First notes"'
  Run 'bolt status'
  Run 'echo "hello again" > notes.txt'
  Run 'bolt status'
  Run 'bolt changes'
  Run 'bolt save "More notes"'
  Run 'bolt history'
  Run 'bolt undo'
  Check ((Get-Content notes.txt) -ceq 'hello again')

  Step 'the guide: removing Bolt from a computer'
  Set-Location -LiteralPath $HOME
  Run 'Remove-Item $HOME\bin\bolt.exe'
  Run 'Remove-Item -Recurse $env:APPDATA\bolt'
  Check (-not (Test-Path -LiteralPath "$env:APPDATA\bolt"))
  InDoc $guide 'Then take the folder off your PATH: open Start, search for "environment variables", open "Edit environment variables for your account", choose `Path`, then "Edit", select the folder and choose "Delete". Leave it if other programs live in it.'
  Check (@(Get-ChildItem -LiteralPath $bin -Force).Count -eq 0)
  # The dialog's work, in the registry.
  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $userPath = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $bin }) -join ';'
  [Environment]::SetEnvironmentVariable('Path', $userPath, 'User')

  Step 'in a new terminal, bolt is no longer found'
  New-TerminalPath
  Check (-not (Get-Command bolt -ErrorAction SilentlyContinue))

  Remove-Item -LiteralPath $reading, $practice, $unpacked -Recurse -Force
  Write-Host ''
  Write-Host 'first-run passed: x86_64-pc-windows-msvc'
} catch {
  Write-Host "first-run FAILED at: $script:current ($_)"
  exit 1
}
