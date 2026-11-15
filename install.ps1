# Phaenna Universal All-In-One Installer (Windows 10 / 11 PowerShell)
# Installs: GPU terminals (WezTerm, Windows Terminal), players (mpv, VLC),
#           yt-dlp, ffmpeg, Git, Rustup, and builds/installs Phaenna.
$ErrorActionPreference = "Stop"

$RepoUrl = "https://github.com/abdullah-binmadhi/Phaenna.git"
$SrcDir  = Join-Path $env:LOCALAPPDATA "phaenna-src"

function Write-Step($msg) {
    Write-Host "`n[phaenna-setup] $msg" -ForegroundColor Cyan
}

if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
    throw "winget is required on Windows 10/11. Install 'App Installer' from the Microsoft Store and re-run."
}

$Packages = @(
    "wez.wezterm",
    "Microsoft.WindowsTerminal",
    "mpv.net",
    "shinchiro.mpv",
    "VideoLAN.VLC",
    "yt-dlp.yt-dlp",
    "Gyan.FFmpeg",
    "Git.Git",
    "Rustlang.Rustup"
)

Write-Step "Installing GPU terminals, video players, yt-dlp, ffmpeg, Git, and Rustup via winget..."
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

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $CargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    if (Test-Path (Join-Path $CargoBin "rustup.exe")) {
        $env:PATH = "$CargoBin;$env:PATH"
    }
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

Write-Step "Running dependency check (phaenna doctor)..."
$PhaennaBin = Join-Path (Join-Path $env:USERPROFILE ".cargo\bin") "phaenna.exe"
if (Test-Path $PhaennaBin) {
    & $PhaennaBin doctor
} else {
    phaenna doctor
}

Write-Host "`nInstallation complete! Open WezTerm or Windows Terminal and run: phaenna" -ForegroundColor Green
