#!/usr/bin/env nu

def main [
] {
    # Get build date and number

    let last_deploy = try {
        open last-deploy.toml
    } catch {
        {date: "2000-00-00" number: 0}
    }

    let today = date now | format date "%Y-%m-%d"
    let number = if $today != $last_deploy.date {
        1
    } else {
        $last_deploy.number + 1
    }
    
    let ext = if $env.OS == Windows_NT { ".exe" }
    
    let configs = [
        {platform: windows target: x86_64-pc-windows-msvc ext: .exe}
        {platform: linux target: x86_64-unknown-linux-gnu ext: null}
    ]

    if (which cross | is-empty) {
        print 'cross-rs is not installed.'
        return
    }

    $configs | each {|config|
        let name = $'($today).($number)-($config.platform).tar.gz'

        cross build --release --target $config.target
        if ("staging" | path exists) { rm -r staging }
        mkdir staging
        cp $'target/($config.target)/release/lobby-server($config.ext)' staging/
        cp $'target/($config.target)/release/game($config.ext)' staging/
        cp -r game/assets staging/assets
        $'($today).($number)' | save staging/version

        let name = $'($today).($number)-($config.platform).tar.gz'
        let latest = $'latest-($config.platform).tar.gz'
        
        cd staging
        tar -czvf $name ./*
        scp $name server:moba/deploy/
        ssh server $"cd moba/deploy; if \('($latest)' | path exists) { rm ($latest) }; ln ($name) ($latest)"
        cd ..
        rm -r staging
    }

    { date: $today, number: $number } | save -f last-deploy.toml
}
