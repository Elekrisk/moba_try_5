use bevy::prelude::*;
use lightyear::prelude::client::{self, Authentication, ClientCommands};
use network::GameServerToken;

pub mod camera;
pub mod map;
pub mod network;

pub fn game(app: &mut App) {
    app.add_plugins((camera::camera, map::map));

    app.add_systems(OnEnter(crate::State::InGame), setup);
}

fn setup(token: Res<GameServerToken>, mut config: ResMut<client::ClientConfig>, mut commands: Commands) {
    let client::NetConfig::Netcode { auth, .. } = &mut config.net else {
        unreachable!()
    };
    *auth = Authentication::Token(token.0.clone());
    commands.connect_client();
}
