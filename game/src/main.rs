#![feature(trivial_bounds)]
#![feature(try_blocks)]
#![feature(never_type)]
#![feature(type_alias_impl_trait)]
#![feature(let_chains)]

mod game;
mod lobby;
mod login;
mod ui;

use std::{
    collections::HashMap,
    net::{Ipv6Addr, SocketAddrV6},
    time::Duration,
};

use bevy::prelude::*;
use bevy_cosmic_edit::CosmicEditPlugin;
use bevy_tokio_tasks::TokioTasksPlugin;
use clap::Parser;
use game::network::{build_client_plugin, build_server_plugin};
use lightyear::prelude::{generate_key, ConnectToken};
use lobby::{lobby, SendMessage};
use lobby_server::{
    ConnectTokenWrapper, MessageFromGameServerToLobby, MessageFromLobbyToGameServer, ReadMessage,
    WriteMessage,
};
use login::{login, LobbyConnection, LoginName};
use tokio::io::AsyncWriteExt;
use ui::ui;
use uuid::Uuid;
use wtransport::{config::Ipv6DualStackConfig, Endpoint, Identity, ServerConfig, VarInt};

#[derive(Debug, clap::Parser)]
struct Options {
    #[command(subcommand)]
    cmd: Option<Subcommand>,

    #[command(flatten)]
    client_args: ClientArgs,
}

#[derive(Debug, clap::Subcommand)]
enum Subcommand {
    Client(ClientArgs),
    Server(ServerArgs),
}

#[derive(Debug, clap::Args)]
struct ClientArgs {
    name: Option<String>,
}

#[derive(Debug, clap::Args)]
struct ServerArgs {
    lobby_server_token: Uuid,
    port: u16,
}

fn main() -> AppExit {
    let options = Options::parse();

    println!("{options:#?}");

    let command = options
        .cmd
        .unwrap_or(Subcommand::Client(options.client_args));

    match command {
        Subcommand::Client(client_args) => client_main(client_args),
        Subcommand::Server(server_args) => server_main(server_args),
    }
}

fn client_main(options: ClientArgs) -> AppExit {
    let mut app = App::new();

    app.add_plugins((
        DefaultPlugins,
        TokioTasksPlugin::default(),
        build_client_plugin(),
    ))
    .insert_state(State::Login)
    .add_plugins(ui)
    .add_plugins(login)
    .add_plugins(lobby)
    .add_plugins(game::game)
    .add_systems(Startup, setup)
    .add_systems(Last, listen_for_exit);

    if let Some(name) = options.name {
        app.insert_resource(LoginName(name));
    }
    app.run()
}

fn server_main(options: ServerArgs) -> AppExit {
    // We need to generate connection tokens for every player
    // To do so, we first need to receive the connection from the lobby server

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

fn listen_for_exit(event_reader: EventReader<AppExit>, send: Option<Res<SendMessage>>) {
    if !event_reader.is_empty()
        && let Some(send) = send
    {
        let _ = send.send(lobby_server::MessageFromPlayer::Disconnecting);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, States)]
enum State {
    Login,
    Lobby,
    InGame,
}

fn setup(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 59.0,
            ..default()
        }),
    ));
}

pub trait FlattenResult<T> {
    fn flatten2(self) -> anyhow::Result<T>;
}

impl<T, E1: Into<anyhow::Error>, E2: Into<anyhow::Error>> FlattenResult<T>
    for Result<Result<T, E1>, E2>
{
    fn flatten2(self) -> anyhow::Result<T> {
        match self {
            Ok(res) => match res {
                Ok(val) => Ok(val),
                Err(e) => Err(e.into()),
            },
            Err(e) => Err(e.into()),
        }
    }
}
