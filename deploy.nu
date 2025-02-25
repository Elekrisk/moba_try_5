#!/usr/bin/env nu

def main [
] {
    # Get last deployed version
    let latest = http get http://server.lan:8081/versions | get version | where $it != latest | sort -n | last

    let latest = $latest | parse '{date}.{number}' | get 0
    let today = date now | format date '%Y-%m-%d'

    let version = if $latest.date == $today {
        $'($today).($latest.number | into int | $in + 1)'
    } else {
        $'($today).1'
    }
    
    let ext = if $nu.os-info.name == windows { ".exe" }
    
    let configs = [
        {platform: windows target: x86_64-pc-windows-gnu ext: .exe}
        {platform: linux target: x86_64-unknown-linux-gnu ext: null}
    ]

    if (which cross | is-empty) {
        print 'cross-rs is not installed.'
        return
    }

    $configs | each {|config|
        let name = $'($version)-($config.platform).tar.gz'

        cross build --release --target $config.target --target-dir $'($config.target)/target'
        if ("staging" | path exists) { rm -r staging }
        mkdir staging
        cp $'($config.target)/target/($config.target)/release/lobby-server($config.ext)' staging/
        cp $'($config.target)/target/($config.target)/release/game($config.ext)' staging/
        cp -r game/assets staging/assets
        $version | save staging/version

        let name = $'($version)-($config.platform).tar.gz'
        let latest = $'latest-($config.platform).tar.gz'
        
        cd staging
        tar -czvf $name ./*
        scp $name server:moba/deploy/
        ssh server $"cd moba/deploy; if \('($latest)' | path exists) { rm ($latest) }; ln ($name) ($latest)"
        cd ..
        rm -r staging
    }
}
