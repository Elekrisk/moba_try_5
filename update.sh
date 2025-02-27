#!/usr/bin/bash

# Update script for linux

set -e

# Read the current version
version=$(<version.txt)
echo "Current version is $version"

# Get latest version
latest_version=`curl -s "https://moba.elekrisk.com/versions/latest/linux" | jq -r '.version.string_rep'`
echo "Latest version is $latest_version"

if [ $version == $latest_version ]; then
    echo "Already at latest version; exiting..."
    exit 0
fi

# Fetch latest version
echo "Downloading latest version..."
curl -s "https://moba.elekrisk.com/download/latest/linux" > latest.tar.gz

# Move current installation into 'last-version' folder
if [ -d last-version ]; then
    rm -r last-version/*
fi
mkdir -p last-version
mv assets game lobby-server update.sh version.txt last-version/

echo "Unpacking..."
# Untar new installation into this same folder
tar xzf latest.tar.gz
# Remove tar.gz
rm latest.tar.gz

echo "Successfully updated."
