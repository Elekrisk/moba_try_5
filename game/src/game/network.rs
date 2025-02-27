use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use bevy::prelude::*;
use client::ClientPlugins;
use engine::shared_config;
use lightyear::{connection::netcode::PRIVATE_KEY_BYTES, prelude::*};
use server::{NetConfig, ServerPlugins};

#[derive(Resource)]
pub struct GameServerToken(pub ConnectToken);

pub fn build_client_plugin() -> ClientPlugins {
    let auth = client::Authentication::None;
    let io = client::IoConfig {
        transport: client::ClientTransport::UdpSocket(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::UNSPECIFIED),
            0,
        )),
        ..default()
    };
    let net_config = client::NetConfig::Netcode {
        auth,
        config: client::NetcodeConfig::default(),
        io,
    };
    let config = client::ClientConfig {
        shared: shared_config(),
        net: net_config,
        ..default()
    };
    ClientPlugins::new(config)
}
