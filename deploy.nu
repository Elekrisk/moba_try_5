#!/usr/bin/env nu

def main [
] {
    # Get last deployed version
    let latest = try {
        http get http://server.lan:8081/versions/latest
    } catch {
        if (http get https://moba.elekrisk.com/versions | is-empty) {
            # No versions available; set latest to a dummy value
            {version: {date: ''}}
        }
    }

    let today = date now | format date '%Y-%m-%d'

    let version = if $latest.version.date == $today {
        $'($today).($latest.version.number | into int | $in + 1)'
    } else {
        $'($today).1'
    }
    
    let configs = [
        {
            platform: windows
            target: x86_64-pc-windows-gnu
            ext: .exe
            archive_ext: .zip
            archive_cmd: {^zip $in *}
            update_script: update.ps1
        }
        {
            platform: linux
            target: x86_64-unknown-linux-gnu
            ext: null
            archive_ext: .tar.gz
            archive_cmd: {^tar -czvf $in *}
            update_script: update.sh
        }
    ]

    if (which cross | is-empty) {
        print 'cross-rs is not installed.'
        return
    }

    $configs | each {|config|
        RUSTFLAGS="" cross build --release --target $config.target --target-dir $'($config.target)/target'
        if ("staging" | path exists) { rm -r staging }
        mkdir staging
        cp $'($config.target)/target/($config.target)/release/lobby-server($config.ext)' staging/
        cp $'($config.target)/target/($config.target)/release/game($config.ext)' staging/
        cp $'($config.target)/target/($config.target)/release/server($config.ext)' staging/
        cp -r assets staging/assets
        cp $config.update_script staging/
        $version | $"($in)\n" | save staging/version.txt

        let name = $'($version)-($config.platform)($config.archive_ext)'
        let latest = $'latest-($config.platform)($config.archive_ext)'
        
        cd staging
        $name | do $config.archive_cmd
        scp -i ../deploy_key $name moba-deploy@server:moba/download/versions/
        ssh -i ../deploy_key moba-deploy@server $"cd moba/download/versions/; [ -f '($latest)' ] && rm ($latest) ; ln ($name) ($latest)"
        cd ..
        rm -r staging
    } | ignore

    echo "Deployment confirmed."
    let lobby_running = ssh -i deploy_key moba-deploy@server systemctl is-active moba-lobby-server -q | complete | $in.exit_code == 0

    def ask [question action] {
        print $'($question) [yN]'
        mut accepted = false
        loop {
            let key = input listen --types [key] | update modifiers {parse 'keymodifiers({mod})' | get -i mod}
            match [$key.code $key.modifiers] {
                ['y' []] | ['Y' ['shift']] => {
                    do $action
                    $accepted = true
                    break
                }
                ['n' []] | ['N' ['shift']] | ['enter' []] | ['c' ['control']] => {break}
                _ => {
                    continue
                }
            }
        }
        $accepted
    }

    if $lobby_running {
        print "Server running."
    } else {
        print "Server not running."
    }

    ask "Do you want to update the server to the published version?" {
        ssh -i deploy_key moba-deploy@server 'cd ~/moba/server/deployment; ./update.sh'
        if $lobby_running {
            ask "Do you want to restart the server?" {
                ssh -i deploy_key moba-deploy@server sudo systemctl restart moba-lobby-server
            }
        } else {
            ask "Do you want to start the server?" {
                ssh -i deploy_key moba-deploy@server sudo systemctl start moba-lobby-server
            }
        }
    } | ignore
}
