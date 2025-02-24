pub mod checkbox;

use bevy::{
    ecs::system::RunSystemOnce,
    input::mouse::{MouseScrollUnit, MouseWheel},
    picking::{focus::HoverMap, PickSet},
    prelude::*,
};
use bevy_cosmic_edit::{
    cosmic_text::{Attrs, Family, FontSystem, Metrics},
    prelude::*,
    utils::{deselect_editor_on_esc, print_editor_text},
    CosmicBackgroundColor, CosmicEditPlugin, CosmicFontConfig, CosmicTextAlign, CursorColor,
    MaxLines, ScrollEnabled,
};

pub fn ui(app: &mut App) {
    app.add_plugins(CosmicEditPlugin {
        font_config: CosmicFontConfig {
            load_system_fonts: true,
            fonts_dir_path: Some("assets/fonts".into()),
            font_bytes: None,
        },
    })
    .add_systems(PreUpdate, scroll_ui.after(PickSet::Focus))
    .add_systems(Update, (print_editor_text, deselect_editor_on_esc));
}

#[derive(Debug, Component)]
pub struct ScrollEvent {
    pub dx: f32,
    pub dy: f32,
}

impl Event for ScrollEvent {
    type Traversal = &'static Parent;

    const AUTO_PROPAGATE: bool = true;
}

fn scroll_ui(
    mut mouse_wheel_events: EventReader<MouseWheel>,
    hover_map: Res<HoverMap>,
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
) {
    for mouse_wheel_event in mouse_wheel_events.read() {
        const LINE_HEIGHT: f32 = 21.0;
        let (mut dx, mut dy) = match mouse_wheel_event.unit {
            MouseScrollUnit::Line => (
                mouse_wheel_event.x * LINE_HEIGHT,
                mouse_wheel_event.y * LINE_HEIGHT,
            ),
            MouseScrollUnit::Pixel => (mouse_wheel_event.x, mouse_wheel_event.y),
        };

        if keyboard_input.pressed(KeyCode::ControlLeft)
            || keyboard_input.pressed(KeyCode::ControlRight)
        {
            std::mem::swap(&mut dx, &mut dy);
        }

        for (_pointer, pointer_map) in hover_map.iter() {
            for (entity, _hit) in pointer_map.iter() {
                commands.trigger_targets(ScrollEvent { dx, dy }, *entity);
            }
        }
    }
}

#[derive(Component)]
struct Modal;

pub struct CloseModal;

impl Command for CloseModal {
    fn apply(self, world: &mut World) {
        world.run_system_once(close_modal).unwrap();
    }
}

fn close_modal(modal: Option<Single<Entity, With<Modal>>>, mut commands: Commands) {
    if let Some(e) = modal {
        commands.entity(*e).despawn_recursive();
    }
}

pub struct CreateModal {
    pub title: String,
    pub allow_close: bool,
    pub builder: Box<dyn FnOnce(&mut ChildBuilder) + Send + Sync + 'static>,
}

impl CreateModal {
    pub fn new(title: impl Into<String>, allow_close: bool, builder: impl FnOnce(&mut ChildBuilder) + Send + Sync + 'static) -> Self {
        Self {
            title: title.into(),
            allow_close,
            builder: Box::new(builder),
        }
    }
}

impl CreateModal {
    pub fn info(content: String) -> Self {
        Self::new("Info", true, move |parent: &mut ChildBuilder| {
            parent.spawn(Text::new(content));
        })
    }
}

impl Command for CreateModal {
    fn apply(self, world: &mut World) {
        let mut commands = world.commands();
        create_modal(&mut commands, self.title, self.allow_close, self.builder);
        world.flush();
    }
}

fn stop_propagation<E>(mut trigger: Trigger<E>) {
    trigger.propagate(false);
}

pub fn create_modal(
    commands: &mut Commands,
    title: impl Into<String>,
    allow_close: bool,
    builder: impl FnOnce(&mut ChildBuilder),
) {
    fn close_modal(
        mut trigger: Trigger<Pointer<Click>>,
        modal: Query<Entity, With<Modal>>,
        mut commands: Commands,
    ) {
        if let Some(e) = modal.iter().last() {
            commands.entity(e).despawn_recursive();
        }
        trigger.propagate(false);
    }

    let mut root = commands
        // Root container
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Modal,
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        ));
    root.with_children(|parent| {
        // Modal itself
        parent
            .spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BorderColor(Color::WHITE),
                BackgroundColor(Color::srgb(0.3, 0.1, 0.1)),
            ))
            .with_children(|parent| {
                // Top bar
                parent
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Row,
                            width: Val::Percent(100.0),
                            padding: UiRect::all(Val::Px(10.0)),
                            border: UiRect::bottom(Val::Px(1.0)),
                            ..default()
                        },
                        BorderColor(Color::WHITE),
                    ))
                    .with_children(|parent| {
                        parent
                            .spawn(Node {
                                flex_grow: 1.0,
                                justify_content: JustifyContent::Center,
                                ..default()
                            })
                            .with_child(Text::new(title));
                        if allow_close {
                            parent
                                .spawn((Button, Text::new("[X]")))
                                .observe(close_modal);
                        }
                    });
                // Contents
                parent
                    .spawn(Node {
                        padding: UiRect::all(Val::Px(10.0)),
                        flex_direction: FlexDirection::Column,
                        ..default()
                    })
                    .with_children(builder);
            })
            .observe(stop_propagation::<Pointer<Click>>);
    });
    if allow_close {
        root.observe(close_modal);
    }
}

pub trait CommandModalExt {
    fn modal(
        &mut self,
        title: impl Into<String>,
        allow_close: bool,
        builder: impl FnOnce(&mut ChildBuilder) + Send + Sync + 'static,
    );
    fn info(&mut self, content: impl Into<String>);
}

impl CommandModalExt for Commands<'_, '_> {
    fn modal(
        &mut self,
        title: impl Into<String>,
        allow_close: bool,
        builder: impl FnOnce(&mut ChildBuilder) + Send + Sync + 'static,
    ) {
        self.queue(CreateModal::new(title, allow_close, builder));
    }

    fn info(&mut self, content: impl Into<String>) {
        self.queue(CreateModal::info(content.into()));
    }
}

pub fn build_textedit(
    parent: &mut ChildBuilder,
    initial: impl AsRef<str>,
    font_system: &mut FontSystem,
) -> Entity {
    let mut attrs = Attrs::new().metrics(Metrics::new(16.0, 16.0));
    attrs = attrs.family(Family::Name("Fira Mono"));
    attrs = attrs.color(CosmicColor::rgb(255, 255, 255));

    parent
        .spawn((
            TextEdit,
            CosmicEditBuffer::default().with_text(font_system, initial.as_ref(), attrs),
            ScrollEnabled::Enabled,
            CosmicTextAlign::left_center(),
            CosmicWrap::InfiniteLine,
            MaxLines(1),
            Node {
                min_width: Val::Px(200.0),
                min_height: Val::Px(6.0),
                // padding: UiRect::top(Val::Px(25.0)),
                ..default()
            },
            CosmicBackgroundColor(Color::srgb(0.1, 0.1, 0.1)),
            CursorColor(Color::WHITE),
        ))
        .observe(focus_on_click)
        .id()
}

pub trait OnClickExt {
    fn on_click<M, S: IntoSystem<(), (), M> + Clone + Send + Sync + 'static>(
        &mut self,
        system: S,
    ) -> &mut Self;
}

impl OnClickExt for EntityCommands<'_> {
    fn on_click<M, S: IntoSystem<(), (), M> + Clone + Send + Sync + 'static>(
        &mut self,
        system: S,
    ) -> &mut Self {
        self.observe(
            move |mut trigger: Trigger<Pointer<Click>>, mut commands: Commands| {
                trigger.propagate(false);
                let system = system.clone();
                commands.queue(move |world: &mut World| {
                    world.run_system_once(system).unwrap();
                });
            },
        )
    }
}
