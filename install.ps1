# Installs Platter for the current user on Windows from a GitHub release: the setup is downloaded, checked
# against the release's SHA256SUMS and run without questions (per user, in %LOCALAPPDATA%\Platter, no
# administrator rights). The installed copy then updates itself.
#
#   irm https://github.com/MSpiechowicz/platter/releases/latest/download/install.ps1 | iex
#
# A given release, or removing Platter again (your scans and settings stay):
#
#   $env:PLATTER_VERSION = '0.2.0'; irm https://github.com/MSpiechowicz/platter/releases/latest/download/install.ps1 | iex
#   $env:PLATTER_UNINSTALL = '1';   irm https://github.com/MSpiechowicz/platter/releases/latest/download/install.ps1 | iex
#
# Saved as a file it also takes parameters: .\install.ps1 -Version 0.2.0, .\install.ps1 -Uninstall.
#
# PLATTER_RELEASE_URL replaces the place the files are downloaded from (an https:// URL or a local folder,
# for testing). PLATTER_INSTALL_DRY_RUN=1 downloads and checks everything but does not run the setup.
#
# Everything runs from Install-Platter on the last line, so a download cut short runs nothing.

param(
    [string]$Version = $env:PLATTER_VERSION,
    [switch]$Uninstall = ($env:PLATTER_UNINSTALL -eq '1')
)

function Install-Platter {
    param([string]$Version, [bool]$Uninstall)

    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    # Windows PowerShell 5.1 may still offer TLS 1.0 first; GitHub needs 1.2.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $installDir = Join-Path $env:LOCALAPPDATA 'Platter'

    if ($Uninstall) {
        $uninstaller = Join-Path $installDir 'uninstall.exe'
        if (-not (Test-Path -LiteralPath $uninstaller -PathType Leaf)) {
            Write-Host "Platter is not installed in $installDir."
            return
        }
        $run = Start-Process -FilePath $uninstaller -ArgumentList '/S' -Wait -PassThru
        if ($run.ExitCode -ne 0) { throw "The uninstaller stopped with exit code $($run.ExitCode)." }
        Write-Host 'Platter was removed. Your scans and settings are still in your profile.'
        return
    }

    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
        throw "Platter is built for 64-bit x86 processors (AMD64), not $($env:PROCESSOR_ARCHITECTURE)."
    }

    if ($env:PLATTER_RELEASE_URL) {
        $base = $env:PLATTER_RELEASE_URL.TrimEnd('/', '\')
    } elseif ($Version) {
        if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { throw "'$Version' is not a version such as 0.2.0." }
        $base = "https://github.com/MSpiechowicz/platter/releases/download/v$Version"
    } else {
        $base = 'https://github.com/MSpiechowicz/platter/releases/latest/download'
    }
    $local = -not ($base -match '^[a-z]+://')
    if (-not $local -and -not $base.StartsWith('https://')) { throw "Only https:// downloads are allowed, not $base." }

    # A folder of its own, so nothing else in TEMP can be taken for the download.
    $work = Join-Path ([IO.Path]::GetTempPath()) ("platter-install-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $work | Out-Null
    try {
        function Get-ReleaseFile([string]$name, [string]$to) {
            if ($local) {
                Copy-Item -LiteralPath (Join-Path $base $name) -Destination $to
            } else {
                Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $to -MaximumRedirection 10
            }
        }

        $sums = Join-Path $work 'SHA256SUMS'
        Get-ReleaseFile 'SHA256SUMS' $sums

        # Exactly one setup in the list: "<64 hex digits>  Platter_<version>_x64-setup.exe".
        $lines = @(Get-Content -LiteralPath $sums | Where-Object { $_ -match '^[0-9a-f]{64}  [A-Za-z0-9._-]+_x64-setup\.exe$' })
        if ($lines.Count -ne 1) { throw "Expected exactly one Windows setup in SHA256SUMS, found $($lines.Count)." }
        $expected, $name = $lines[0] -split '  ', 2

        $setup = Join-Path $work $name
        Write-Host "Downloading $name..."
        Get-ReleaseFile $name $setup

        $actual = (Get-FileHash -LiteralPath $setup -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($actual -ne $expected) { throw "$name does not match SHA256SUMS. Nothing was installed." }
        Write-Host "Checked $name against SHA256SUMS."

        if ($env:PLATTER_INSTALL_DRY_RUN -eq '1') {
            Write-Host 'Dry run: the setup was not run.'
            return
        }

        # /S: the NSIS setup asks nothing and installs for the current user only.
        $run = Start-Process -FilePath $setup -ArgumentList '/S' -Wait -PassThru
        if ($run.ExitCode -ne 0) { throw "The setup stopped with exit code $($run.ExitCode)." }
        Write-Host "Platter is installed in $installDir. Start it from the Start menu."
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Install-Platter -Version $Version -Uninstall $Uninstall.IsPresent
