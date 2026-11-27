# Phaenna Universal All-In-One Installer (Windows 10 / 11 PowerShell)
# Installs: mpv, yt-dlp, deno (YouTube JS solver), ffmpeg, GPU terminal (WezTerm / Windows Terminal if missing),
#           and Phaenna (prebuilt GitHub Release binary with SHA256 verification, or -FromSource).
param(
    [switch]$FromSource
)

$ErrorActionPreference = "Stop"

$RepoSlug    = "abdullah-binmadhi/Phaenna"
$RepoUrl     = "https://github.com/$RepoSlug.git"
$ReleaseBase = "https://github.com/$RepoSlug/releases/latest/download"
$BinDir      = Join-Path $env:LOCALAPPDATA "Programs\phaenna"
$SrcDir      = Join-Path $env:LOCALAPPDATA "phaenna-src"

function Write-Step($msg) {
    Write-Host "`n[phaenna-setup] $msg" -ForegroundColor Cyan
}

function Add-UserPathEntry($dir) {
    $currentUserPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if (-not $currentUserPath) { $currentUserPath = "" }
    $parts = $currentUserPath.Split(';') | Where-Object { $_ -ne "" }
    if ($parts -notcontains $dir) {
        $next = ($parts + $dir) -join ';'
        [Environment]::SetEnvironmentVariable("Path", $next, "User")
    }
    if (($env:PATH.Split(';') | Where-Object { $_ -ne "" }) -notcontains $dir) {
        $env:PATH = "$dir;$env:PATH"
    }
}

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    throw "winget is required on Windows 10/11. Install 'App Installer' from the Microsoft Store and re-run."
}

$Packages = @(
    "shinchiro.mpv",
    "yt-dlp.yt-dlp",
    "DenoLand.Deno",
    "Gyan.FFmpeg"
)

if (-not (Get-Command wezterm -ErrorAction SilentlyContinue)) {
    $Packages += "wez.wezterm"
}
if (-not (Get-Command wt -ErrorAction SilentlyContinue)) {
    $Packages += "Microsoft.WindowsTerminal"
}

Write-Step "Installing mpv, yt-dlp, deno, ffmpeg, and terminal tools via winget..."
foreach ($pkg in $Packages) {
    Write-Host " -> Ensuring $pkg..."
    try {
        winget install --id $pkg -e --silent --accept-package-agreements --accept-source-agreements 2>$null | Out-Null
    } catch {
        Write-Host "    (skipped or already installed: $pkg)" -ForegroundColor DarkGray
    }
}

# Refresh PATH in the current PowerShell session so newly installed tools resolve immediately
$env:PATH = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" +
            [System.Environment]::GetEnvironmentVariable("Path", "User")

$InstalledExe = Join-Path $BinDir "phaenna.exe"
$UseSource = $FromSource -or ((Test-Path ".\Cargo.toml") -and ((Get-Content ".\Cargo.toml" -Raw) -match 'name = "phaenna"'))

$PrebuiltOk = $false
if (-not $UseSource -and ([Environment]::Is64BitOperatingSystem)) {
    $Asset = "phaenna-windows-x86_64.exe"
    $TmpDir = Join-Path $env:TEMP ("phaenna-dl-" + [System.Guid]::NewGuid().ToString("N"))
    New-Item -ItemType Directory -Force -Path $TmpDir | Out-Null
    $TmpExe  = Join-Path $TmpDir $Asset
    $TmpSums = Join-Path $TmpDir "SHA256SUMS.txt"
    try {
        Write-Step "Downloading prebuilt release binary ($Asset)..."
        Invoke-WebRequest -Uri "$ReleaseBase/$Asset" -OutFile $TmpExe -UseBasicParsing
        $HashOk = $true
        try {
            Invoke-WebRequest -Uri "$ReleaseBase/SHA256SUMS.txt" -OutFile $TmpSums -UseBasicParsing
            $Line = Get-Content $TmpSums | Where-Object { $_ -match "\s$([regex]::Escape($Asset))$" } | Select-Object -First 1
            if ($Line) {
                $Expected = ($Line -split '\s+')[0].ToLowerInvariant()
                $Actual   = (Get-FileHash -Path $TmpExe -Algorithm SHA256).Hash.ToLowerInvariant()
                if ($Expected -ne $Actual) {
                    Write-Warning "Checksum mismatch for $Asset; falling back to source build."
                    $HashOk = $false
                }
            }
        } catch {
            # Continue if checksum manifest isn't attached
        }
        if ($HashOk) {
            New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
            Move-Item -Force -Path $TmpExe -Destination $InstalledExe
            Add-UserPathEntry $BinDir
            $PrebuiltOk = $true
        }
    } catch {
        Write-Warning "Prebuilt release unavailable; falling back to source build."
    } finally {
        Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $TmpDir
    }
}

if (-not $PrebuiltOk) {
    foreach ($pkg in @("Git.Git", "Rustlang.Rustup")) {
        Write-Host " -> Ensuring $pkg..."
        try {
            winget install --id $pkg -e --silent --accept-package-agreements --accept-source-agreements 2>$null | Out-Null
        } catch {}
    }
    $CargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    if (Test-Path $CargoBin) { Add-UserPathEntry $CargoBin }
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        if (Get-Command rustup -ErrorAction SilentlyContinue) {
            Write-Step "Initializing default stable Rust toolchain..."
            rustup default stable
        } else {
            throw "Rust toolchain (cargo/rustup) not found in PATH after installation. Restart PowerShell and re-run."
        }
    }

    $BuildDir = $null
    if ((Test-Path ".\Cargo.toml") -and ((Get-Content ".\Cargo.toml" -Raw) -match 'name = "phaenna"')) {
        $BuildDir = (Get-Location).Path
        Write-Step "Building Phaenna from local checkout ($BuildDir)..."
    } else {
        Write-Step "Cloning Phaenna into $SrcDir..."
        if (Test-Path (Join-Path $SrcDir ".git")) {
            git -C $SrcDir pull --ff-only
        } else {
            git clone $RepoUrl $SrcDir
        }
        $BuildDir = $SrcDir
    }
    Write-Step "Compiling and installing phaenna..."
    cargo install --path $BuildDir --force
    $InstalledExe = Join-Path $CargoBin "phaenna.exe"
}

Write-Step "Running dependency check (phaenna doctor)..."
if (Test-Path $InstalledExe) {
    & $InstalledExe doctor
} else {
    phaenna doctor
}

Write-Host "`nInstallation complete! Open WezTerm or Windows Terminal and run: phaenna" -ForegroundColor Green
