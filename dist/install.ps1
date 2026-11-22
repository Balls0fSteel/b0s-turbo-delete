# Installs the latest Turbo Delete release for the current user.
#
#     irm https://raw.githubusercontent.com/Balls0fSteel/b0s-turbo-delete/master/dist/install.ps1 | iex

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is very slow with a progress bar
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$url = 'https://github.com/Balls0fSteel/b0s-turbo-delete/releases/latest/download/turbo-delete-setup.exe'
$installer = Join-Path $env:TEMP 'turbo-delete-setup.exe'

Write-Host 'Downloading Turbo Delete' -ForegroundColor Cyan
Invoke-WebRequest -Uri $url -OutFile $installer -UseBasicParsing

try {
    Write-Host 'Installing Turbo Delete' -ForegroundColor Cyan
    $process = Start-Process -FilePath $installer -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "Installer failed with exit code $($process.ExitCode)"
    }
} finally {
    Remove-Item $installer -ErrorAction SilentlyContinue
}

Write-Host 'Installed Turbo Delete. Open a new terminal to use `td`, or right-click a folder in Explorer.' -ForegroundColor Green
