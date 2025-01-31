use std::{net::{IpAddr, Ipv6Addr, SocketAddr}, time::Duration};

use bevy::prelude::*;
use client::ClientPlugins;
use lightyear::prelude::*;
use server::{NetConfig, ServerPlugins};

pub const FIXED_TIMESTEP_HZ: f64 = 64.0;
pub const SERVER_REPLICATION_INTERVAL: Duration = Duration::from_millis(100);

pub fn shared_config() -> SharedConfig {
    SharedConfig {
        server_replication_send_interval: SERVER_REPLICATION_INTERVAL,
        tick: TickConfig { tick_duration: Duration::from_secs_f64(1.0 / FIXED_TIMESTEP_HZ) },
        mode: Mode::Separate,
    }
}

pub fn build_client_plugin(address: SocketAddr) -> ClientPlugins {
    let auth = client::Authentication::Manual {
        server_addr: address,
        client_id: 0,
        private_key: Key::default(),
        protocol_id: 0,
    };
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

pub fn build_server_plugin() -> ServerPlugins {
    let io = server::IoConfig {
        transport: server::ServerTransport::UdpSocket(SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 35475)),
        ..default()
    };
    let net_config = server::NetConfig::Netcode { config: server::NetcodeConfig::default(), io };
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
