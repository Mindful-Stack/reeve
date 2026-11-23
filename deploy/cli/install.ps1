# Installs the reeve CLI on Windows from the Reeve server that served this script (kb/ADR-0030). The server replaces
# the quoted placeholder below with its public_url when it starts.
#
#   [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12; irm https://<server>/cli/install.ps1 | iex
#
# (The prefix turns TLS 1.2 on for Windows PowerShell 5.1, which may not offer it; PowerShell 7 does not need it.)
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
    $Rerun = "[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12; irm $ReeveUrl/cli/install.ps1 | iex"

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
    # The first line `reeve --version` prints, or '' when it does not run. Native stderr is made harmless first: in
    # Windows PowerShell 5.1 a native command's stderr becomes a terminating error under $ErrorActionPreference = 'Stop'.
    function Get-ReeveVersion([string] $Exe) {
        $old = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        try {
            $text = & $Exe --version 2>$null | Out-String
            if ($LASTEXITCODE -ne 0) { return '' }
            return @($text -split "`r?`n")[0]
        } catch {
            return ''
        } finally {
            $ErrorActionPreference = $old
        }
    }
    try {
        # <server>/cli/<name> into $tmp. No credentials are sent; a redirect is either an error or a non-200 answer,
        # and both fail. A loopback download never goes through a proxy: a configured one would carry plain http off
        # the machine, where the binary and its checksum could both be replaced (kb/ADR-0030 R5). PowerShell 6 and
        # later have -NoProxy; Windows PowerShell 5.1 has not, and takes the proxy from [Net.WebRequest]::DefaultWebProxy
        # when the request is made, so that is emptied for the one call and put back after it.
        function Get-CliFile([string] $Name) {
            $dest = Join-Path $tmp $Name
            $request = @{ UseBasicParsing = $true; Uri = "$ReeveUrl/cli/$Name"; OutFile = $dest; PassThru = $true; MaximumRedirection = 0 }
            $loopback = $ReeveUrl.StartsWith('http://', [StringComparison]::OrdinalIgnoreCase)
            $swapped = $false
            $savedProxy = $null
            try {
                if ($loopback -and $PSVersionTable.PSVersion.Major -ge 6) {
                    $request.NoProxy = $true
                } elseif ($loopback) {
                    $savedProxy = [Net.WebRequest]::DefaultWebProxy
                    $swapped = $true
                    [Net.WebRequest]::DefaultWebProxy = $null
                }
                $r = Microsoft.PowerShell.Utility\Invoke-WebRequest @request
            } catch {
                $code = 0
                try { $code = [int] $_.Exception.Response.StatusCode } catch { $code = 0 }
                if ($code -eq 429) {
                    throw "reeve install: downloading ${Name}: the server's proxy is rate-limiting /cli/; wait a minute and rerun"
                }
                if ($code -ge 500 -and $code -le 599) {
                    throw "reeve install: downloading ${Name}: the server answered HTTP $code; it may be restarting: wait a minute and rerun"
                }
                throw "reeve install: downloading $Name failed: $($_.Exception.Message)"
            } finally {
                if ($swapped) { [Net.WebRequest]::DefaultWebProxy = $savedProxy }
            }
            if ($r.StatusCode -ne 200) {
                throw "reeve install: downloading ${Name}: the server answered HTTP $($r.StatusCode)"
            }
            $dest
        }

        # The target first, so a run that would refuse it downloads nothing.
        $elsewhere = "To install somewhere else, rerun with:`n  `$env:REEVE_INSTALL_DIR = '<directory>'; $Rerun"
        if (-not $env:REEVE_INSTALL_DIR -and -not $env:LOCALAPPDATA) {
            throw "reeve install: LOCALAPPDATA is not set, so there is no default directory. $elsewhere"
        }
        $dir = if ($env:REEVE_INSTALL_DIR) { $env:REEVE_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\reeve' }
        # A relative directory means relative to the current location, not to the process's working directory.
        $dir = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($dir)
        $target = Join-Path $dir 'reeve.exe'
        if (Test-Path -LiteralPath $target) {
            $first = Get-ReeveVersion $target
            if ("$first" -notlike 'reeve-planner*' -and $env:REEVE_FORCE -ne '1') {
                throw "reeve install: $target is not the Reeve planner CLI (it printed: $first). To replace it, rerun with:`n  `$env:REEVE_FORCE = '1'; $Rerun"
            }
        }
        try {
            New-Item -ItemType Directory -Force -Path $dir | Out-Null
            # Staged beside the target (same volume), so the replacement below is a rename, never a copy. Created empty
            # now, which proves the directory writable before anything is downloaded.
            $staged = Join-Path $dir ('.reeve.new.' + [Guid]::NewGuid() + '.exe')
            New-Item -ItemType File -Path $staged | Out-Null
        } catch {
            throw "reeve install: cannot write to ${dir}: $($_.Exception.Message). $elsewhere"
        }

        # The build the server offers: a 40-character CLI version, shown by its first seven.
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
            # Mid-deploy the sums and the binary can come from two releases: a rerun gets one release's pair.
            throw "reeve install: checksum mismatch for ${file}: nothing was installed. Rerun; if it repeats, tell whoever runs the server"
        }

        try {
            Copy-Item -Force -LiteralPath $bin -Destination $staged
        } catch {
            throw "reeve install: cannot write to ${dir}: $($_.Exception.Message). $elsewhere"
        }
        $ran = Get-ReeveVersion $staged
        if ("$ran" -notlike 'reeve-planner*') {
            throw 'reeve install: the downloaded reeve.exe does not run on this machine; nothing was replaced'
        }
        if (Test-Path -LiteralPath $target) {
            # Never delete-then-move: File.Replace swaps the staged file in and keeps the old one as a backup; on any
            # failure the existing reeve.exe is left (or put back) in place.
            $backup = Join-Path $dir ('.reeve.old.' + [Guid]::NewGuid() + '.exe')
            try {
                [IO.File]::Replace($staged, $target, $backup, $true)
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
        # The user's Path lives in the registry as REG_EXPAND_SZ with %VAR% entries; GetEnvironmentVariable would expand
        # them and SetEnvironmentVariable would write the expanded text back, freezing every entry. Read the raw
        # string, compare against expanded entries, append to the raw string and write it back as REG_EXPAND_SZ.
        # A failure here must not undo or hide a finished install: warn with the manual step instead.
        $envKey = $null
        try {
            $envKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
            if (-not $envKey) { $envKey = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment') }
            $raw = [string] $envKey.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            # Keep REG_SZ if that is what the value was; otherwise (or when absent) REG_EXPAND_SZ.
            $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
            if ($null -ne $envKey.GetValue('Path', $null) -and $envKey.GetValueKind('Path') -eq [Microsoft.Win32.RegistryValueKind]::String) {
                $kind = [Microsoft.Win32.RegistryValueKind]::String
            }
            $have = @($raw -split ';' | Where-Object { $_ } | ForEach-Object { [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') })
            if ($have -notcontains $dir.TrimEnd('\')) {
                $new = if ($raw) { $raw.TrimEnd(';') + ';' + $dir } else { $dir }
                $envKey.SetValue('Path', $new, $kind)
                # Setting and clearing a dummy user variable broadcasts WM_SETTINGCHANGE, so new windows see the Path.
                [Environment]::SetEnvironmentVariable('REEVE_PATH_BROADCAST', '1', 'User')
                [Environment]::SetEnvironmentVariable('REEVE_PATH_BROADCAST', [NullString]::Value, 'User')
                Write-Host "added $dir to your user Path (open terminals need reopening)"
            }
        } catch {
            Write-Warning "could not add $dir to your user Path: $($_.Exception.Message). Add it in Settings > System > About > Advanced system settings > Environment Variables, or run reeve by its full path: $target"
        } finally {
            if ($envKey) { $envKey.Close() }
        }
        Write-Host "installed reeve build $($build.Substring(0, 7)) at $target"
        # Another reeve.exe ahead of $dir in a new window's Path (the machine Path first, then the user's) would run
        # instead of this one, silently. Best effort: a Path entry that cannot be read is skipped.
        try {
            $order = @(((@([Environment]::GetEnvironmentVariable('Path', 'Machine'), [Environment]::GetEnvironmentVariable('Path', 'User')) -join ';') -split ';') |
                Where-Object { $_ } | ForEach-Object { [Environment]::ExpandEnvironmentVariables($_).TrimEnd('\') })
            foreach ($entry in $order) {
                if ($entry -eq $dir.TrimEnd('\')) { break }
                $other = $null
                try { $other = Join-Path $entry 'reeve.exe' } catch { continue }
                if (Test-Path -LiteralPath $other -PathType Leaf) {
                    Write-Warning "another reeve.exe comes first on your Path: $other (it prints: $(Get-ReeveVersion $other)). Remove it, or move $dir ahead of $entry in your Path; until then run $target"
                    break
                }
            }
        } catch {
        }
        Write-Host ''
        Write-Host "next: reeve auth login --url $ReeveUrl"
        Write-Host "  (or: & '$target' auth login --url $ReeveUrl)"
        Write-Host 'then: reeve tui'
    } finally {
        if ($staged) { Remove-Item -Force -LiteralPath $staged -ErrorAction SilentlyContinue }
        Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
    }
}
