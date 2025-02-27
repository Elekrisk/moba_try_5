# Update script for windows

# Read the current version
$version = Get-Content ./version.txt
Write-Output "Current version is $version"

# Get latest version
$latest_version = (Invoke-WebRequest "https://moba.elekrisk.com/versions/latest/linux" | ConvertFrom-Json).version.string_rep
Write-Output "Latest version is $latest_version"

if ($version -Eq $latest_version) {
    Write-Output "Already at latest version; exiting..."
    exit 0
}

# Fetch latest version
Write-Output "Downloading latest version..."
Invoke-WebRequest "https://moba.elekrisk.com/donwload/latest/windows" -OutFile "latest.zip"

# Move current installation into 'last-version' folder
if (Test-Path -Path "last-version") {
    Remove-Item "last-version/*" -Recurse
} else {
    New-Item -Path . -Name "last-version" -ItemType "directory"
}
Move-Item -Path assets,game.exe,server.exe,lobby-server.exe,update.ps1,version.txt -Destination last-version/

# Unzip new intallation into this same folder
Write-Output "Unpacking..."
Expand-Archive -Path latest.zip -Destination . -Force
# Remove .zip
Remove-Item latest.zip

Write-Output "Successfully updated."
