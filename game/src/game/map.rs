use bevy::prelude::*;

use super::camera::CameraTarget;

pub fn map(app: &mut App) {
    app.add_systems(OnEnter(crate::State::InGame), spawn_map);
}

pub fn spawn_map(assets: Res<AssetServer>, mut commands: Commands) {
    commands.spawn((
        Mesh3d(assets.add(Plane3d::new(Vec3::Y, Vec2::new(50.0, 50.0)).into())),
        MeshMaterial3d(assets.add(StandardMaterial { ..default() })),
    ));

    commands.spawn((Transform::from_translation(Vec3::ZERO), CameraTarget));
}
