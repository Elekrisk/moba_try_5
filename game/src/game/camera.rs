use bevy::prelude::*;

#[derive(Component)]
pub struct CameraTarget;

pub fn camera(app: &mut App) {
    app.add_systems(Update, camera_follow_target);
}

fn camera_follow_target(
    mut camera: Single<&mut Transform, With<Camera>>,
    target: Option<Single<&Transform, (With<CameraTarget>, Without<Camera>)>>,
) {
    let Some(target) = target else { return };

    let offset = Vec3::new(0.0, 10.0, 4.0);
    camera.translation = target.translation + offset;
    camera.look_at(target.translation, Dir3::Y);
}
