#!/usr/bin/env nu

# Check for new update

print 'Checking for new version...'

let url = 'http://server.lan:8081'

let platform = match (uname).kernel-name {
    Windows_NT => 'windows'
    Linux => 'linux'
    _ => 'unknown'
}

let ext = if $platform == windows { '.exe' }

let last = http get $url
    | lines
    | parse -r '>(?<file>[^<]+)</a'
    | get file
    | parse '{date}.{number}-{platform}.tar.gz'
    | where platform == $platform
    | if ($in | is-not-empty) { last | each {$'($in.date).($in.number)'} }

if $last == null {
    print $'No version for platform ($platform) found.'
    return
}

if (try { open version | str trim } catch { '' }) == $last {
    print 'Newest version already downloaded.'
    return
}

print $"New version ($last) found! \(current is (open version | str trim))"

if $platform == windows {    
    let game_running = powershell -c 'wmic process get ProcessID,ExecutablePath'
        | from ssv
        | where ExecutablePath == ("./game.exe" | path expand)
        | is-not-empty
    
    if $game_running {
        print 'Cannot update while game is running; please exit the game and try again'
        return
    }
}

print 'Do you want to update?'

if ([Update Cancel] | input list | default Cancel) == Cancel {
    return
}

print 'Backing up previous version...'

if ("last-version" | path exists) {
    print 'Previous backup deleted'
    rm -r last-version
}
mkdir last-version
cp -r assets game.exe lobby-server.exe last-version/

print 'Downloading...'

http get $'($url)/latest-($platform).tar.gz' | save new-version.tar.gz -f

print 'Extracting...'
rm -r assets $'game($ext)' $'lobby-server($ext)'
tar -xzvf new-version.tar.gz
rm new-version.tar.gz

print 'Updated!'
if $platform == windows {
    print 'Press any key to continue...'
    input listen -t [key] | ignore
}
