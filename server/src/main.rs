use std::{
    collections::HashMap,
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6},
    time::Duration,
};

use bevy::prelude::*;
use clap::Parser;
use engine::{SERVER_REPLICATION_INTERVAL, shared_config};
use lightyear::{
    connection::netcode::PRIVATE_KEY_BYTES, prelude::*, server::plugin::ServerPlugins,
};
use lobby_server::{
    ConnectTokenWrapper, MessageFromGameServerToLobby, MessageFromLobbyToGameServer, ReadMessage,
    WriteMessage,
};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use wtransport::{Endpoint, Identity, ServerConfig, config::Ipv6DualStackConfig};

#[derive(Debug, clap::Parser)]
struct ServerArgs {
    lobby_server_token: Uuid,
    port: u16,
}

fn main() -> AppExit {
    // We need to generate connection tokens for every player
    // To do so, we first need to receive the connection from the lobby server

    let options = ServerArgs::parse();

    println!("GS: Generating key...");
    let key = generate_key();

    // Start listening server
    let builder = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    builder.block_on(async move {
        let server = Endpoint::server(
            ServerConfig::builder()
                .with_bind_address_v6(
                    SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, options.port, 0, 0),
                    Ipv6DualStackConfig::Allow,
                )
                .with_identity(Identity::self_signed(["localhost", "127.0.0.1", "::1"]).unwrap())
                .keep_alive_interval(Some(Duration::from_secs(15)))
                .build(),
        )
        .unwrap();

        let conn = server.accept().await.await.unwrap().accept().await.unwrap();
        let mut stream = conn.accept_uni().await.unwrap();
        let connect_message: MessageFromLobbyToGameServer = stream.read_message().await.unwrap();

        println!("GS: Received LS message!");
        let MessageFromLobbyToGameServer::LobbyInitialMessage { token, players } = connect_message;

        if token != options.lobby_server_token {
            // Wrong server
            panic!("Wrong server token");
        }

        // Generate connection token for every player

        println!("GS: Key generated!");

        let mut tokens = HashMap::new();

        for id in players.values().flatten().map(|sel| sel.player.id) {
            println!("GS: Generating token...");
            let token = ConnectToken::build(
                format!("localhost:{}", options.port),
                0,
                id.get().as_u64_pair().0,
                key,
            )
            .timeout_seconds(15)
            .generate()
            .unwrap();

            let wrapped_token = ConnectTokenWrapper(token.try_into_bytes().unwrap().into());

            tokens.insert(id, wrapped_token);
            println!("GS: Token generated!");
        }

        println!("GS: Writing message...");
        let mut stream = conn.open_uni().await.unwrap().await.unwrap();
        stream
            .write_message_framed(MessageFromGameServerToLobby::PlayerTokensGenerated {
                players: tokens,
            })
            .await
            .unwrap();
        stream.flush().await.unwrap();
        stream.finish().await.unwrap();
        println!("GS: Message written!");
    });

    App::new()
        .add_plugins((MinimalPlugins, build_server_plugin(key)))
        .run()
}

pub fn build_server_plugin(private_key: [u8; PRIVATE_KEY_BYTES]) -> ServerPlugins {
    let io = server::IoConfig {
        transport: server::ServerTransport::UdpSocket(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            35475,
        )),
        ..default()
    };
    let config = server::NetcodeConfig {
        private_key,
        ..default()
    };

    let net_config = server::NetConfig::Netcode { config, io };
    let config = server::ServerConfig {
        shared: shared_config(),
        net: vec![net_config],
        replication: ReplicationConfig {
            send_interval: SERVER_REPLICATION_INTERVAL,
            ..default()
        },
        ..default()
    };
    ServerPlugins::new(config)
}
