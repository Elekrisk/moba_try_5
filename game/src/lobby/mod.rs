mod champ_select;

use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use bevy::{ecs::component::StorageType, prelude::*, utils::HashMap};
use bevy_cosmic_edit::{
    cosmic_text::{Attrs, AttrsOwned, BufferRef, Edit as _, FontSystem},
    editor::CosmicEditor,
    input::drag,
    BufferRefExtras as _, CosmicEditBuffer, CosmicFontSystem,
};
use bevy_tokio_tasks::TokioTasksRuntime;
use champ_select::build_champ_select;
use lightyear::{
    client::config::{ClientConfig, NetcodeConfig},
    prelude::{
        client::{Authentication, ClientTransport, IoConfig, NetConfig}, ConnectToken, SharedConfig
    },
};
use lobby_server::{
    Lobby, LobbyId, LobbySettings, LobbyShortInfo, LobbyState as LState, MessageFromPlayer,
    MessageFromServer, PlayerId, PlayerInfo, ReadMessage, Team, WriteMessage,
};
use tokio::task::JoinHandle;

use crate::{
    game::network::GameServerToken, login::{LobbyConnection, MyPlayerId}, ui::{
        build_textedit,
        checkbox::{build_checkbox, Checkbox},
        create_modal, CloseModal, CreateModal, OnClickExt, ScrollEvent,
    }
};

pub fn lobby(app: &mut App) {
    app.enable_state_scoped_entities::<crate::State>();
    app.add_sub_state::<LobbyState>();
    app.add_event::<MsgEvent>();
    app.init_resource::<PlayerCache>();
    app.init_resource::<PlayerSlotAnchorMap>();
    app.add_observer(on_msg_send);
    app.add_observer(refresh_lobby_list);
    app.add_observer(refresh_lobby_interface);
    app.add_observer(on_player_info_updated);
    app.add_systems(OnEnter(crate::State::Lobby), (setup, setup_ui));
    app.add_systems(OnExit(crate::State::Lobby), cleanup);
    app.add_systems(OnEnter(LobbyState::LobbyBrowser), on_enter_lobby_browser);
    app.add_systems(OnEnter(LobbyState::InLobby), on_enter_lobby);
}

#[derive(Debug, PartialEq, Eq, Clone, Copy, Hash, SubStates, Default)]
#[source(crate::State = crate::State::Lobby)]
enum LobbyState {
    #[default]
    LobbyBrowser,
    InLobby,
    ConnectingToGameServer,
}

#[derive(Event)]
pub struct MsgEvent(MessageFromServer);

#[derive(Resource)]
pub struct RecvLobbyTask(JoinHandle<()>);

#[derive(Resource)]
pub struct SendLobbyTask(JoinHandle<()>);

#[derive(Resource)]
pub struct CurrentLobby {
    id: LobbyId,
    info: Option<Lobby>,
}

#[derive(Resource, Deref)]
pub struct SendMessage(tokio::sync::mpsc::UnboundedSender<MessageFromPlayer>);

fn setup(
    runtime: Res<TokioTasksRuntime>,
    connection: Res<LobbyConnection>,
    mut commands: Commands,
) {
    let conn = connection.0.clone();
    let reciever = runtime.spawn_background_task(|mut ctx| async move {
        let Err(e): anyhow::Result<!> = try {
            loop {
                let mut stream = conn.accept_uni().await?;
                let message = stream.read_message().await?;
                ctx.run_on_main_thread(move |ctx| {
                    info!("Message received: {message:?}");
                    ctx.world.trigger(MsgEvent(message));
                })
                .await;
            }
        };
        warn!("Error receiving lobby message: {e}");
        ctx.run_on_main_thread(move |ctx| {
            ctx.world
                .trigger(MsgEvent(MessageFromServer::ServerShutdown));
        })
        .await;
    });

    let (send, mut recv) = tokio::sync::mpsc::unbounded_channel();

    let conn = connection.0.clone();
    let sender = runtime.spawn_background_task(|_| async move {
        let x: anyhow::Result<()> = try {
            while let Some(msg) = recv.recv().await {
                let should_exit = matches!(msg, MessageFromPlayer::Disconnecting);
                info!("Message sent: {msg:?}");
                conn.open_uni().await?.await?.write_message(msg).await?;
                if should_exit {
                    break;
                }
            }
        };
        if let Err(e) = x {
            warn!("Error sending lobby message: {e}");
        }
    });

    commands.insert_resource(RecvLobbyTask(reciever));
    commands.insert_resource(SendLobbyTask(sender));
    commands.insert_resource(SendMessage(send));
}

fn cleanup(recv_task: Res<RecvLobbyTask>, send_task: Res<SendLobbyTask>, mut commands: Commands) {
    recv_task.0.abort();
    send_task.0.abort();
    commands.remove_resource::<RecvLobbyTask>();
    commands.remove_resource::<SendLobbyTask>();
    commands.remove_resource::<SendMessage>();
}

#[derive(Component)]
struct LobbyTabAnchor;

fn setup_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            StateScoped(crate::State::Lobby),
        ))
        .with_children(|parent| {
            // Top bar

            // Lobby tab anchor
            parent.spawn((
                Node {
                    width: Val::Percent(100.0),
                    max_height: Val::Percent(100.0),
                    flex_grow: 1.0,
                    ..default()
                },
                LobbyTabAnchor,
            ));
            // build_lobby_list(parent);
        });
}

fn on_enter_lobby_browser(
    lobby_tab_anchor: Single<Entity, With<LobbyTabAnchor>>,
    send: Res<SendMessage>,
    mut commands: Commands,
) {
    // Create lobby list
    commands
        .entity(*lobby_tab_anchor)
        .despawn_descendants()
        .with_children(build_lobby_list);
    // Send request to fetch lobbies
    // The response message triggers building the list
    let _ = send.send(MessageFromPlayer::GetLobbyList);
}

#[derive(Component)]
struct LobbyListAnchor;

fn build_lobby_list(parent: &mut ChildBuilder) {
    // Root container
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|parent| {
            // Button bar
            parent
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    ..default()
                })
                .with_children(|parent| {
                    // Create new lobby button
                    parent.spawn((Button, Text::new("[Create Lobby]"))).observe(
                        |mut trigger: Trigger<Pointer<Click>>, res: Res<SendMessage>| {
                            let _ = res.0.send(MessageFromPlayer::CreateLobby);
                            trigger.propagate(false);
                        },
                    );

                    // Refresh lobby list button
                    parent.spawn((Button, Text::new("[Refresh]"))).observe(
                        |mut trigger: Trigger<Pointer<Click>>, res: Res<SendMessage>| {
                            let _ = res.0.send(MessageFromPlayer::GetLobbyList);
                            trigger.propagate(false);
                        },
                    );
                });

            // Lobby list anchor
            parent
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        width: Val::Percent(100.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    LobbyListAnchor,
                ))
                .with_children(|parent| {
                    for i in 0..100 {
                        build_lobby_list_entry(
                            parent,
                            &LobbyShortInfo {
                                id: LobbyId::new(),
                                name: format!("Lobby {i}"),
                                player_count: 0,
                                max_player_count: 10,
                            },
                        );
                    }
                })
                .observe(scroll);
        });
}

#[derive(Event)]
struct RefreshLobbyList(Vec<LobbyShortInfo>);

fn refresh_lobby_list(
    trigger: Trigger<RefreshLobbyList>,
    lobby_anchor: Single<Entity, With<LobbyListAnchor>>,
    mut commands: Commands,
) {
    let mut entity_commands = commands.entity(*lobby_anchor);
    entity_commands
        .despawn_descendants()
        .with_children(|parent| {
            for lobby in &trigger.0 {
                build_lobby_list_entry(parent, lobby);
            }
        });
}

fn build_lobby_list_entry(parent: &mut ChildBuilder, lobby_info: &LobbyShortInfo) {
    parent
        .spawn(Node {
            padding: UiRect::all(Val::Px(2.0)),
            column_gap: Val::Px(10.0),
            ..default()
        })
        .with_children(|parent| {
            parent.spawn((
                Text::new(&lobby_info.name),
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
            ));
            parent.spawn(Text::new(format!(
                "{}/{}",
                lobby_info.player_count, lobby_info.max_player_count
            )));
            let id = lobby_info.id;
            parent.spawn((Button, Text::new("[Join]"))).observe(
                move |mut trigger: Trigger<Pointer<Click>>, send: Res<SendMessage>| {
                    let _ = send.send(MessageFromPlayer::JoinLobby(id));
                    trigger.propagate(false);
                },
            );
        });
}

fn scroll(mut trigger: Trigger<ScrollEvent>, mut q: Query<&mut ScrollPosition>) {
    info!("Scroll!");
    let event = trigger.event();
    if let Ok(mut scroll) = q.get_mut(trigger.entity()) {
        scroll.offset_x -= event.dx;
        scroll.offset_y -= event.dy;
        trigger.propagate(false);
    }
}

fn on_enter_lobby(
    lobby_tab_anchor: Single<Entity, With<LobbyTabAnchor>>,
    current_lobby: Res<CurrentLobby>,
    send: Res<SendMessage>,
    mut commands: Commands,
) {
    // Create lobby interface
    commands
        .entity(*lobby_tab_anchor)
        .despawn_descendants()
        .with_children(build_lobby_interface);

    let _ = send.send(MessageFromPlayer::GetLobbyInfo(current_lobby.id));
}

#[derive(Component)]
struct LobbyInterfaceAnchor;

fn build_lobby_interface(parent: &mut ChildBuilder) {
    parent.spawn((
        Node {
            width: Val::Percent(100.0),
            max_height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        LobbyInterfaceAnchor,
    ));
}

#[derive(Event)]
struct RefreshLobbyInterface;

struct LobbyBuildingContext<'a> {
    lobby: &'a Lobby,
    player_cache: &'a PlayerCache,
    send: &'a SendMessage,
    my_id: PlayerId,
}

impl LobbyBuildingContext<'_> {
    fn i_am_leader(&self) -> bool {
        self.lobby.leader == self.my_id
    }

    fn my_team(&self) -> Team {
        self.lobby
            .players
            .iter()
            .find(|(_, v)| v.contains(&self.my_id))
            .map(|(t, _)| *t)
            .unwrap()
    }
}

fn refresh_lobby_interface(
    _trigger: Trigger<RefreshLobbyInterface>,
    lobby: Res<CurrentLobby>,
    lobby_interface_anchor: Single<Entity, With<LobbyInterfaceAnchor>>,
    player_cache: Res<PlayerCache>,
    send: Res<SendMessage>,
    my_id: Res<MyPlayerId>,
    mut commands: Commands,
) {
    commands
        .entity(*lobby_interface_anchor)
        .despawn_descendants()
        .with_children(|parent| {
            update_lobby_interface(
                &LobbyBuildingContext {
                    lobby: lobby.info.as_ref().unwrap(),
                    player_cache: &player_cache,
                    send: &send,
                    my_id: my_id.0,
                },
                parent,
            )
        });
}

fn update_lobby_interface(ctx: &LobbyBuildingContext, parent: &mut ChildBuilder) {
    match &ctx.lobby.lobby_state {
        LState::Normal => { /* TODO: break out into separate module instead of continuing in this function */
        }
        LState::ChampSelect(_) => {
            build_champ_select(ctx, parent);
            return;
        }
        LState::InGame => unreachable!("Should never need to draw UI for this"),
    }

    // Top bar
    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|parent| {
            // Exit lobby button
            parent
                .spawn((Button, Text::new("[Exit Lobby]")))
                .on_click(|send: Res<SendMessage>| {
                    let _ = send.send(MessageFromPlayer::LeaveLobby);
                });

            if ctx.lobby.leader == ctx.my_id {
                // Settings button
                let settings = ctx.lobby.settings.clone();
                parent
                    .spawn((Button, Text::new("[Lobby Settings]")))
                    .on_click(
                        move |mut font_system: ResMut<CosmicFontSystem>, mut commands: Commands| {
                            create_modal(&mut commands, "Lobby Settings", false, |parent| {
                                build_settings_menu(&settings, &mut font_system.0, parent)
                            });
                        },
                    );
                parent
                    .spawn((Button, Text::new("[Enter Champ Select]")))
                    .on_click(move |send: Res<SendMessage>| {
                        let _ = send.send(MessageFromPlayer::EnterChampSelect);
                    });
            }
        });

    // Team list
    // Layout is a vertical list of team pairs
    parent
        .spawn(Node {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            overflow: Overflow::scroll_y(),
            ..default()
        })
        .observe(scroll)
        .with_children(|parent| {
            for i in 0..ctx.lobby.settings.team_count / 2 {
                let left_team = Team(i * 2);
                let right_team = Team(i * 2 + 1);

                parent
                    .spawn(Node {
                        width: Val::Percent(100.0),
                        ..default()
                    })
                    .with_children(|parent| {
                        build_team_list(left_team, ctx, parent);
                        build_team_list(right_team, ctx, parent);
                    });
            }

            if ctx.lobby.settings.team_count % 2 == 1 {
                parent
                    .spawn(Node {
                        width: Val::Percent(100.0),
                        ..default()
                    })
                    .with_children(|parent| {
                        build_team_list(Team(ctx.lobby.settings.team_count - 1), ctx, parent);
                    });
            }
        });
}

fn build_settings_menu(
    settings: &LobbySettings,
    font_system: &mut FontSystem,
    parent: &mut ChildBuilder,
) {
    // Lobby name
    fn row(
        parent: &mut ChildBuilder,
        label: impl Into<String>,
        builder: impl FnOnce(&mut ChildBuilder) -> Entity,
    ) -> Entity {
        let mut res = Entity::PLACEHOLDER;
        parent
            .spawn(Node {
                width: Val::Percent(100.0),
                ..default()
            })
            .with_children(|parent| {
                parent.spawn(Text::new(label));
                res = builder(parent);
            });
        res
    }
    let lobby_name = row(parent, "Lobby Name: ", |parent| {
        build_textedit(parent, &settings.name, font_system)
    });
    let allow_joining = row(parent, "Allow joining: ", |parent| {
        build_checkbox(parent, settings.lobby_is_open)
    });
    let can_change_team = row(parent, "Players can change Team: ", |parent| {
        build_checkbox(parent, settings.players_can_change_team)
    });
    let map = row(parent, "Map: ", |parent| {
        build_textedit(parent, &settings.map, font_system)
    });
    let team_count = row(parent, "Teams: ", |parent| {
        let d = parent.spawn((Button, Text::new("[-] "))).id();
        let t = parent
            .spawn(Text::new(settings.team_count.to_string()))
            .id();
        let u = parent.spawn((Button, Text::new(" [+]"))).id();

        parent.enqueue_command(move |world: &mut World| {
            world.spawn(
                Observer::new(
                    move |mut trigger: Trigger<Pointer<Click>>, mut q: Query<&mut Text>| {
                        trigger.propagate(false);
                        let mut text = q.get_mut(t).unwrap();
                        text.0 = 1.max(text.0.parse::<usize>().unwrap() - 1).to_string();
                    },
                )
                .with_entity(d),
            );
            world.spawn(
                Observer::new(
                    move |mut trigger: Trigger<Pointer<Click>>, mut q: Query<&mut Text>| {
                        trigger.propagate(false);
                        let mut text = q.get_mut(t).unwrap();
                        text.0 = (text.0.parse::<usize>().unwrap() + 1).to_string();
                    },
                )
                .with_entity(u),
            );
        });
        t
    });
    let players_per_team = row(parent, "Players per Team: ", |parent| {
        let d = parent.spawn((Button, Text::new("[-] "))).id();
        let t = parent
            .spawn(Text::new(settings.player_limit_per_team.to_string()))
            .id();
        let u = parent.spawn((Button, Text::new(" [+]"))).id();

        parent.enqueue_command(move |world: &mut World| {
            world.spawn(
                Observer::new(
                    move |mut trigger: Trigger<Pointer<Click>>, mut q: Query<&mut Text>| {
                        trigger.propagate(false);
                        let mut text = q.get_mut(t).unwrap();
                        text.0 = 1.max(text.0.parse::<usize>().unwrap() - 1).to_string();
                    },
                )
                .with_entity(d),
            );
            world.spawn(
                Observer::new(
                    move |mut trigger: Trigger<Pointer<Click>>, mut q: Query<&mut Text>| {
                        trigger.propagate(false);
                        let mut text = q.get_mut(t).unwrap();
                        text.0 = (text.0.parse::<usize>().unwrap() + 1).to_string();
                    },
                )
                .with_entity(u),
            );
        });
        t
    });

    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            ..default()
        })
        .with_children(|parent| {
            parent
                .spawn((Button, Text::new("[Cancel]")))
                .on_click(|mut commands: Commands| {
                    commands.queue(CloseModal);
                });
            parent.spawn((Button, Text::new("[Save]"))).on_click(
                move |bq: Query<&CosmicEditBuffer>,
                      eq: Query<&CosmicEditor>,
                      tq: Query<&Text>,
                      cq: Query<&Checkbox>,
                      send: Res<SendMessage>,
                      mut commands: Commands| {
                    let get_text = |e| match eq.get(e) {
                        Ok(x) => match x.editor.buffer_ref() {
                            BufferRef::Owned(buffer) => buffer.get_text(),
                            BufferRef::Borrowed(buffer) => buffer.get_text(),
                            BufferRef::Arc(buffer) => buffer.get_text(),
                        },
                        Err(_) => bq
                            .get(e)
                            .unwrap()
                            .get_text_spans(AttrsOwned::new(Attrs::new()))
                            .into_iter()
                            .map(|l| l.into_iter().map(|(t, _)| t).collect::<String>())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    };

                    let lobby_name = get_text(lobby_name);
                    let lobby_is_open = cq.get(allow_joining).unwrap().checked;
                    let players_can_change_team = cq.get(can_change_team).unwrap().checked;
                    let map = get_text(map);
                    let team_count = tq.get(team_count).unwrap().0.parse().unwrap();
                    let player_limit_per_team =
                        tq.get(players_per_team).unwrap().0.parse().unwrap();

                    let settings = LobbySettings {
                        name: lobby_name,
                        map,
                        team_count,
                        player_limit_per_team,
                        players_can_change_team,
                        lobby_is_open,
                    };

                    let _ = send.send(MessageFromPlayer::UpdateSettings(settings));
                    commands.queue(CloseModal);
                },
            );
        });
}

fn build_team_list(team: Team, ctx: &LobbyBuildingContext, parent: &mut ChildBuilder) {
    let players = ctx.lobby.players.get(&team).unwrap();
    parent
        .spawn(Node {
            width: Val::Percent(50.0),
            flex_direction: FlexDirection::Column,
            ..default()
        })
        .with_children(|parent| {
            // Team title
            parent.spawn(Node { ..default() }).with_children(|parent| {
                parent.spawn((Node { ..default() }, Text::new(team.to_string())));
                if team != ctx.my_team()
                    && ctx.lobby.players.get(&team).unwrap().len()
                        < ctx.lobby.settings.player_limit_per_team
                    && (ctx.lobby.settings.players_can_change_team || ctx.i_am_leader())
                {
                    let player_id = ctx.my_id;
                    parent.spawn((Button, Text::new("[Move]"))).on_click(
                        move |send: Res<SendMessage>| {
                            let _ = send.send(MessageFromPlayer::SwitchTeam(player_id, team));
                        },
                    );
                }
            });

            // Player slots
            for (index, player) in players
                .iter()
                .copied()
                .map(Some)
                .chain(
                    std::iter::repeat(None).take(
                        ctx.lobby
                            .settings
                            .player_limit_per_team
                            .saturating_sub(players.len()),
                    ),
                )
                .enumerate()
            {
                build_player_slot(team, index, player, ctx, parent);
            }
        });
}

#[derive(Resource, Default)]
struct PlayerSlotAnchorMap {
    player_to_key: HashMap<PlayerId, (Team, usize)>,
    player_to_entity: HashMap<PlayerId, Entity>,
    key_to_entity: HashMap<(Team, usize), Entity>,
}

struct PlayerSlotAnchor {
    team: Team,
    index: usize,
    player: Option<PlayerId>,
}

impl Component for PlayerSlotAnchor {
    const STORAGE_TYPE: StorageType = StorageType::Table;

    fn register_component_hooks(hooks: &mut bevy::ecs::component::ComponentHooks) {
        hooks.on_insert(|mut world, e, _| {
            let data = world.entity(e).get::<Self>().unwrap();
            let key = (data.team, data.index);
            if let Some(player) = data.player {
                world
                    .resource_mut::<PlayerSlotAnchorMap>()
                    .player_to_key
                    .insert(player, key);
                world
                    .resource_mut::<PlayerSlotAnchorMap>()
                    .player_to_entity
                    .insert(player, e);
            }
            world
                .resource_mut::<PlayerSlotAnchorMap>()
                .key_to_entity
                .insert(key, e);
        });
        hooks.on_replace(|mut world, e, _| {
            let data = world.entity(e).get::<Self>().unwrap();
            let key = (data.team, data.index);
            if let Some(player) = data.player {
                world
                    .resource_mut::<PlayerSlotAnchorMap>()
                    .player_to_key
                    .remove(&player);
                world
                    .resource_mut::<PlayerSlotAnchorMap>()
                    .player_to_entity
                    .remove(&player);
            }
            world
                .resource_mut::<PlayerSlotAnchorMap>()
                .key_to_entity
                .remove(&key);
        });
    }
}

#[derive(Resource, Default)]
struct PlayerCache {
    players: HashMap<PlayerId, PlayerInfo>,
}

#[derive(Event)]
struct PlayerInfoUpdated(PlayerInfo);

#[derive(Resource)]
struct DraggedPlayer {
    id: PlayerId,
    entity: Entity,
    pos: Vec2,
    prevent_pos_reset: bool,
}

fn build_player_slot(
    team: Team,
    index: usize,
    player: Option<PlayerId>,
    ctx: &LobbyBuildingContext,
    parent: &mut ChildBuilder,
) {
    let mut ec = parent.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(25.0),
            border: UiRect::all(Val::Px(1.0)),
            column_gap: Val::Px(10.0),
            ..default()
        },
        BorderColor(Color::WHITE),
        PlayerSlotAnchor {
            team,
            index,
            player,
        },
    ));

    let id = ec.id();

    if let Some(player) = player {
        let name = if ctx.player_cache.players.contains_key(&player) {
            ctx.player_cache.players.get(&player).unwrap().name.clone()
        } else {
            let _ = ctx.send.send(MessageFromPlayer::GetPlayerInfo(player));
            "Loading...".into()
        };

        ec.with_children(|parent| build_player_slot_contents(name, player, ctx, parent));

        if player == ctx.my_id || ctx.i_am_leader() {
            ec.observe(
                move |mut trigger: Trigger<Pointer<DragStart>>, mut commands: Commands| {
                    commands.insert_resource(DraggedPlayer {
                        id: player,
                        entity: id,
                        pos: default(),
                        prevent_pos_reset: false,
                    });
                    commands.entity(id).insert(PickingBehavior::IGNORE);
                    trigger.propagate(false);
                },
            );
            ec.observe(
                move |mut trigger: Trigger<Pointer<Drag>>,
                      mut dragged_player: ResMut<DraggedPlayer>,
                      mut q: Query<&mut Node>| {
                    let dist = trigger.event().distance;
                    let mut node = q.get_mut(id).unwrap();
                    node.left = Val::Px(dist.x);
                    node.top = Val::Px(dist.y);
                    dragged_player.pos = dist;
                    trigger.propagate(false);
                },
            );
            ec.observe(
                move |mut trigger: Trigger<Pointer<DragEnd>>,
                      dragged_player: Res<DraggedPlayer>,
                      mut q: Query<&mut Node>,
                      mut commands: Commands| {
                    if !dragged_player.prevent_pos_reset {
                        let mut node = q.get_mut(id).unwrap();
                        node.left = Val::Auto;
                        node.top = Val::Auto;
                    }
                    commands.remove_resource::<DraggedPlayer>();
                    commands.entity(id).remove::<PickingBehavior>();
                    trigger.propagate(false);
                },
            );
            if ctx.i_am_leader() {
                ec.observe(
                    move |mut trigger: Trigger<Pointer<DragDrop>>,
                          dragged_player: Option<ResMut<DraggedPlayer>>,
                          send: Res<SendMessage>| {
                        let Some(mut dragged_player) = dragged_player else {
                            return;
                        };
                        dragged_player.prevent_pos_reset = true;
                        let _ =
                            send.send(MessageFromPlayer::SwitchPlaces(dragged_player.id, player));
                        trigger.propagate(false);
                    },
                );
            }
        }
    } else if ctx.lobby.settings.players_can_change_team || ctx.i_am_leader() {
        ec.observe(
            move |mut trigger: Trigger<Pointer<DragDrop>>,
                  dragged_player: Option<ResMut<DraggedPlayer>>,
                  send: Res<SendMessage>| {
                let Some(mut dragged_player) = dragged_player else {
                    return;
                };
                dragged_player.prevent_pos_reset = true;
                let _ = send.send(MessageFromPlayer::SwitchTeam(dragged_player.id, team));
                trigger.propagate(false);
            },
        );
    }
}

fn build_player_slot_contents(
    name: String,
    player: PlayerId,
    ctx: &LobbyBuildingContext,
    parent: &mut ChildBuilder,
) {
    if ctx.lobby.leader == player {
        parent.spawn((Text::new("[L]"), PickingBehavior::IGNORE));
    }

    parent.spawn((
        Node {
            flex_grow: 1.0,
            ..default()
        },
        Text::new(name),
        PickingBehavior::IGNORE,
    ));

    if ctx.i_am_leader() && player != ctx.my_id {
        parent.spawn((Button, Text::new("[Kick]"))).observe(
            move |mut trigger: Trigger<Pointer<Click>>, send: Res<SendMessage>| {
                trigger.propagate(false);
                let _ = send.send(MessageFromPlayer::KickPlayer(player));
            },
        );
    }
}

fn on_player_info_updated(
    trigger: Trigger<PlayerInfoUpdated>,
    lobby: Res<CurrentLobby>,
    mut cache: ResMut<PlayerCache>,
    send: Res<SendMessage>,
    my_id: Res<MyPlayerId>,
    slot_map: Res<PlayerSlotAnchorMap>,
    mut commands: Commands,
) {
    let info = trigger.event().0.clone();

    let ctx = &LobbyBuildingContext {
        lobby: lobby.info.as_ref().unwrap(),
        player_cache: &cache,
        send: &send,
        my_id: my_id.0,
    };

    if let Some(e) = slot_map.player_to_entity.get(&info.id) {
        commands
            .entity(*e)
            .despawn_descendants()
            .with_children(|parent| {
                build_player_slot_contents(info.name.clone(), info.id, ctx, parent)
            });
    }

    cache.players.insert(info.id, info);
}

fn on_msg_send(
    trigger: Trigger<MsgEvent>,
    current_state: Res<State<LobbyState>>,
    mut next_state: ResMut<NextState<LobbyState>>,
    mut next_game_state: ResMut<NextState<crate::State>>,
    send: Res<SendMessage>,
    current_lobby: Option<Res<CurrentLobby>>,
    mut commands: Commands,
) {
    let event = &trigger.event().0;

    match event {
        MessageFromServer::RequestRefused(msg) => {
            create_modal(&mut commands, "Message from Server", true, |parent| {
                parent.spawn(Text::new(msg));
            });
        }
        MessageFromServer::LobbyList(list) => {
            commands.trigger(RefreshLobbyList(list.clone()));
        }
        MessageFromServer::YouJoinedLobby(id) => {
            commands.insert_resource(CurrentLobby {
                id: *id,
                info: None,
            });
            next_state.set(LobbyState::InLobby);
        }
        MessageFromServer::YouLeftLobby => {
            commands.remove_resource::<CurrentLobby>();
            next_state.set(LobbyState::LobbyBrowser);
        }
        MessageFromServer::LobbyInfo(lobby) => {
            if *current_state == LobbyState::InLobby {
                commands.insert_resource(CurrentLobby {
                    id: lobby.id,
                    info: Some(lobby.clone()),
                });
                commands.trigger(RefreshLobbyInterface);
            }
        }
        MessageFromServer::PlayerInfo(player) => {
            commands.trigger(PlayerInfoUpdated(player.clone()));
        }
        MessageFromServer::PlayerJoinedYourLobby(_)
        | MessageFromServer::LobbyLeaderChanged(_)
        | MessageFromServer::PlayerLeftYourLobby(_)
        | MessageFromServer::PlayerSwitchedTeam(_, _)
        | MessageFromServer::SettingsUpdated(_)
        | MessageFromServer::PlayersSwitched(_, _)
        | MessageFromServer::ChampSelectEntered
        | MessageFromServer::PlayerSelectedChampion(_, _)
        | MessageFromServer::ChampSelectionLocked(_) => {
            let _ = send.send(MessageFromPlayer::GetLobbyInfo(current_lobby.unwrap().id));
        }
        MessageFromServer::GameStarted(address) => {
            let token = ConnectToken::try_from_bytes(&address.0).unwrap();
            info!("Token received");
            commands.insert_resource(GameServerToken(token));
            next_game_state.set(crate::State::InGame);
        },
        MessageFromServer::ServerShutdown => {
            commands.queue(CreateModal::info("Lobby server was shut down".into()));
            next_game_state.set(crate::State::Login);
        }
        MessageFromServer::InitialHandshakeResponse { .. } => unreachable!(),
    }
}
