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
use game::network::build_client_plugin;
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
    name: Option<String>,
}

fn main() -> AppExit {
    let options = Options::parse();

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
