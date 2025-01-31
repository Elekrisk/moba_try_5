use bevy::prelude::*;

#[derive(Component)]
pub struct Checkbox {
    pub checked: bool,
}

pub fn build_checkbox(parent: &mut ChildBuilder, initial: bool) -> Entity {
    parent
        .spawn((
            Button,
            Checkbox { checked: initial },
            Text::new(if initial { "[X]" } else { "[ ]" }),
        ))
        .observe(
            |mut trigger: Trigger<Pointer<Click>>, mut q: Query<(&mut Checkbox, &mut Text)>| {
                trigger.propagate(false);
                let (mut checkbox, mut text) = q.get_mut(trigger.entity()).unwrap();
                checkbox.checked = !checkbox.checked;
                text.0 = if checkbox.checked { "[X]" } else { "[ ]" }.into();
            },
        )
        .id()
}
