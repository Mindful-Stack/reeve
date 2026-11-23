# Installs the reeve CLI on Windows from the Reeve server that served this script (kb/ADR-0030). The server replaces
# the quoted placeholder below with its public_url when it starts.
#
#   irm https://<server>/cli/install.ps1 | iex
#
# $env:REEVE_INSTALL_DIR chooses the directory (default %LOCALAPPDATA%\Programs\reeve); $env:REEVE_FORCE = '1'
# replaces a reeve.exe there that is not the Reeve planner CLI. Everything runs in one script block, so `irm | iex`
# leaves the session's variables and preferences alone (only Path changes), and a download cut short is a parse error
# that runs nothing. Failures `throw`; `exit` would close the window.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    # A local, empty copy: the session's $PSDefaultParameterValues could otherwise give Invoke-WebRequest -Headers,
    # -Credential, -UseDefaultCredentials or -WebSession (kb/ADR-0030 R5: no credentials).
    $PSDefaultParameterValues = @{}
    $ReeveUrl = '@REEVE_URL@'
    $Rerun = "irm $ReeveUrl/cli/install.ps1 | iex"

    # https only; plain http only to a loopback address, for local testing. Checked before anything is downloaded.
    if ($ReeveUrl -notmatch '^https://' -and $ReeveUrl -notmatch '^http://(127\.0\.0\.1|localhost|\[::1\])(:\d+)?$') {
        throw "reeve install: refusing ${ReeveUrl}: downloads go over https only (plain http only to a loopback address)"
    }
    if (-not [Environment]::Is64BitOperatingSystem) {
        throw 'reeve install: no build for 32-bit Windows. Supported: Windows x86_64 (and Windows on arm64, under emulation)'
    }
    # Windows PowerShell 5.1 may not offer TLS 1.2 unless asked.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $file = 'reeve-windows-x86_64.exe'
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ('reeve-' + [Guid]::NewGuid())
    New-Item -ItemType Directory -Path $tmp | Out-Null
    $staged = $null
    try {
        # <server>/cli/<name> into $tmp. No credentials are sent; a redirect is either an error or a non-200 answer,
        # and both fail.
        function Get-CliFile([string] $Name) {
            $dest = Join-Path $tmp $Name
            try {
                $r = Invoke-WebRequest -UseBasicParsing -Uri "$ReeveUrl/cli/$Name" -OutFile $dest -PassThru -MaximumRedirection 0
            } catch {
                throw "reeve install: downloading $Name failed: $($_.Exception.Message)"
            }
            if ($r.StatusCode -ne 200) {
                throw "reeve install: downloading ${Name}: the server answered HTTP $($r.StatusCode)"
            }
            $dest
        }

        # The build the server offers: a 40-character commit id, shown by its first seven.
        $build = (Get-Content -LiteralPath (Get-CliFile 'VERSION') -TotalCount 1)
        if ("$build" -cnotmatch '^[0-9a-f]{40}$') {
            throw "reeve install: the server's VERSION is not a build ($build): is this a Reeve server with CLI distribution on?"
        }
        $sums = Get-CliFile 'SHA256SUMS'
        $lines = @(Get-Content -LiteralPath $sums | Where-Object { ($_ -split '\s+')[1] -eq $file })
        if ($lines.Count -ne 1) {
            throw "reeve install: SHA256SUMS has $($lines.Count) lines for $file, want exactly 1"
        }
        $want = ($lines[0] -split '\s+')[0].ToLowerInvariant()
        $bin = Get-CliFile $file
        $got = (Get-FileHash -Algorithm SHA256 -LiteralPath $bin).Hash.ToLowerInvariant()
        if ($got -ne $want) {
            throw "reeve install: checksum mismatch for ${file}: nothing was installed"
        }

        $dir = if ($env:REEVE_INSTALL_DIR) { $env:REEVE_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\reeve' }
        $target = Join-Path $dir 'reeve.exe'
        if (Test-Path -LiteralPath $target) {
            $first = try { & $target --version 2>$null | Select-Object -First 1 } catch { '' }
            if ("$first" -notlike 'reeve-planner*' -and $env:REEVE_FORCE -ne '1') {
                throw "reeve install: $target is not the Reeve planner CLI (it printed: $first). To replace it, rerun with:`n  `$env:REEVE_FORCE = '1'; $Rerun"
            }
        }
        $elsewhere = "To install somewhere else, rerun with:`n  `$env:REEVE_INSTALL_DIR = '<directory>'; $Rerun"
        try {
            New-Item -ItemType Directory -Force -Path $dir | Out-Null
            # Staged beside the target (same volume), so the replacement below is a rename, never a copy.
            $staged = Join-Path $dir ('.reeve.new.' + [Guid]::NewGuid() + '.exe')
            Copy-Item -LiteralPath $bin -Destination $staged
        } catch {
            throw "reeve install: cannot write to ${dir}: $($_.Exception.Message). $elsewhere"
        }
        $ran = try { & $staged --version 2>$null | Select-Object -First 1 } catch { '' }
        if ("$ran" -notlike 'reeve-planner*') {
            throw 'reeve install: the downloaded reeve.exe does not run on this machine; nothing was replaced'
        }
        if (Test-Path -LiteralPath $target) {
            # Never delete-then-move: File.Replace swaps the staged file in and keeps the old one as a backup; on any
            # failure the existing reeve.exe is left (or put back) in place.
            $backup = Join-Path $dir ('.reeve.old.' + [Guid]::NewGuid() + '.exe')
            try {
                [IO.File]::Replace($staged, $target, $backup)
            } catch {
                if (-not (Test-Path -LiteralPath $target) -and (Test-Path -LiteralPath $backup)) {
                    [IO.File]::Move($backup, $target)
                }
                throw "reeve install: cannot replace $target (is reeve running? close it and rerun): $($_.Exception.Message); nothing was replaced"
            }
            $staged = $null
            # A running old reeve.exe keeps its backup open; the next install removes it.
            Remove-Item -Force -LiteralPath $backup -ErrorAction SilentlyContinue
        } else {
            [IO.File]::Move($staged, $target)
            $staged = $null
        }
        Get-ChildItem -LiteralPath $dir -Filter '.reeve.old.*.exe' -ErrorAction SilentlyContinue |
            Remove-Item -Force -ErrorAction SilentlyContinue

        # This session first, so `reeve` here is the one just installed; the user's Path for new windows.
        $env:Path = (@($dir) + @($env:Path -split ';' | Where-Object { $_ -and $_ -ne $dir })) -join ';'
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if (-not (@($userPath -split ';') -contains $dir)) {
            $new = if ($userPath) { "$userPath;$dir" } else { $dir }
            [Environment]::SetEnvironmentVariable('Path', $new, 'User')
            Write-Host "added $dir to your user Path (open terminals need reopening)"
        }
        Write-Host "installed reeve build $($build.Substring(0, 7)) at $target"
        Write-Host ''
        Write-Host "next: reeve auth login --url $ReeveUrl"
        Write-Host "  (or: & '$target' auth login --url $ReeveUrl)"
        Write-Host 'then: reeve tui'
    } finally {
        if ($staged) { Remove-Item -Force -LiteralPath $staged -ErrorAction SilentlyContinue }
        Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
    }
}
