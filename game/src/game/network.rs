use std::{net::{IpAddr, Ipv6Addr, SocketAddr}, time::Duration};

use bevy::prelude::*;
use client::ClientPlugins;
use lightyear::{connection::netcode::PRIVATE_KEY_BYTES, prelude::*};
use server::{NetConfig, ServerPlugins};

pub const FIXED_TIMESTEP_HZ: f64 = 64.0;
pub const SERVER_REPLICATION_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Resource)]
pub struct GameServerToken(pub ConnectToken);

pub fn shared_config() -> SharedConfig {
    SharedConfig {
        server_replication_send_interval: SERVER_REPLICATION_INTERVAL,
        tick: TickConfig { tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ) },
        mode: Mode::Separate,
    }
}

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

pub fn build_server_plugin(private_key: [u8; PRIVATE_KEY_BYTES]) -> ServerPlugins {
    let io = server::IoConfig {
        transport: server::ServerTransport::UdpSocket(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 35475)),
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
