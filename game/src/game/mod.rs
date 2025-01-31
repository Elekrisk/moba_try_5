use bevy::prelude::*;

pub mod camera;
pub mod map;
pub mod network;

pub fn game(app: &mut App) {
    app.add_plugins((camera::camera, map::map));
}
