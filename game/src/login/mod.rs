use std::{
    net::{Ipv6Addr, SocketAddrV6},
    sync::mpsc::TryRecvError,
    time::Duration,
};

use crate::{lobby::SendMessage, ui::CommandModalExt, FlattenResult, State};
use bevy::prelude::*;
use bevy_cosmic_edit::{
    cosmic_text::{Attrs, AttrsOwned, BufferRef, Color as CosmicColor, Edit, Family, Metrics},
    editor::CosmicEditor,
    prelude::{focus_on_click, TextEdit},
    BufferRefExtras, CosmicBackgroundColor, CosmicEditBuffer, CosmicFontSystem, CosmicTextAlign,
    CosmicWrap, FocusedWidget, MaxLines, ScrollEnabled,
};
use bevy_tokio_tasks::TokioTasksRuntime;
use lobby_server::{
    MessageFromPlayer, MessageFromServer, PlayerId, ReadMessage, WriteMessage as _,
};
use wtransport::{config::Ipv6DualStackConfig, ClientConfig, Connection, Endpoint};

pub fn login(app: &mut App) {
    app.add_sub_state::<LoginState>();
    app.enable_state_scoped_entities::<LoginState>();

    app.add_event::<ConnectionSuccessful>();
    app.add_event::<ConnectionFailed>();

    app.add_systems(OnEnter(LoginState::Login), (cleanup, setup_ui));
    app.add_systems(OnEnter(LoginState::Connecting), setup_connecting);
    app.add_systems(
        Update,
        wait_for_connection_event.run_if(in_state(LoginState::Connecting)),
    );

    app.add_observer(on_successful_connection);
    app.add_observer(on_failed_connection);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SubStates, Default)]
#[source(State = State::Login)]
enum LoginState {
    #[default]
    Login,
    Connecting,
}

#[derive(Resource)]
pub struct LoginName(pub String);

#[derive(Resource)]
struct LoginServer(String);

#[derive(Event)]
struct ConnectionSuccessful;
#[derive(Event)]
struct ConnectionFailed;

#[derive(Resource)]
pub struct MyPlayerId(pub PlayerId);

#[derive(Resource)]
pub struct LobbyConnection(pub Connection);

enum ConnectionEvent {
    ConnectionSuccessful(Connection, PlayerId),
    ConnectionFailed,
}

#[derive(Resource)]
struct ConnectionEventChannel(std::sync::mpsc::Receiver<ConnectionEvent>);

fn cleanup(send: Option<Res<SendMessage>>) {
    if let Some(send) = send {
        let _ = send.send(MessageFromPlayer::Disconnecting);
    }
}

fn build_ui_root<'a>(state: LoginState, commands: &'a mut Commands) -> EntityCommands<'a> {
    let mut ret = commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            ..default()
        },
        StateScoped(state),
    ));
    // ret.observe(
    //     |_: Trigger<Pointer<Click>>, mut focus: ResMut<InputFocus>| {
    //         focus.clear();
    //     },
    // );
    ret
}

fn setup_ui(
    login_name: Option<Res<LoginName>>,
    login_server: Option<Res<LoginServer>>,
    mut font_system: ResMut<CosmicFontSystem>,
    mut commands: Commands,
) {
    let login_server = login_server
        .map(|r| r.0.clone())
        .unwrap_or_else(|| "localhost".into());
    let login_name = login_name
        .map(|r| r.0.clone())
        .unwrap_or_else(|| "Guest".into());

    let mut server_field = Entity::PLACEHOLDER;
    let mut username_field = Entity::PLACEHOLDER;

    let mut attrs = Attrs::new().metrics(Metrics::new(16.0, 16.0));
    attrs = attrs.family(Family::Name("Fira Mono"));
    attrs = attrs.color(CosmicColor::rgb(255, 255, 255));

    build_ui_root(LoginState::Login, &mut commands)
        .with_children(|parent| {
            parent.spawn(Node { ..default() }).with_children(|parent| {
                parent.spawn(Text::new("Server: "));
                server_field = parent
                    .spawn((
                        TextEdit,
                        CosmicEditBuffer::default().with_text(&mut font_system, &login_server, attrs),
                        ScrollEnabled(true),
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
                    ))
                    .observe(focus_on_click)
                    .id();
            });
            parent.spawn(Node { ..default() }).with_children(|parent| {
                parent.spawn(Text::new("Username: "));
                username_field = parent
                    .spawn((
                        TextEdit,
                        CosmicEditBuffer::default().with_text(&mut font_system, &login_name, attrs),
                        ScrollEnabled(true),
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
                    ))
                    .observe(focus_on_click)
                    .id();
            });
            parent
                .spawn((Button, ))
                .with_child(Text::new("Connect"))
                .observe(
                    move |mut trigger: Trigger<Pointer<Click>>,
                          q: Query<&CosmicEditBuffer>,
                          q2: Query<&CosmicEditor>,
                          mut next_state: ResMut<NextState<LoginState>>,
                          mut commands: Commands| {
                        trigger.propagate(false);
                        info!("Clicked button!");

                        let get_text = |e| {
                            match q2.get(e) {
                                Ok(x) => match x.editor.buffer_ref() {
                                    BufferRef::Owned(buffer) => buffer.get_text(),
                                    BufferRef::Borrowed(buffer) => buffer.get_text(),
                                    BufferRef::Arc(buffer) => buffer.get_text(),
                                },
                                Err(_) => q.get(e).unwrap().get_text_spans(AttrsOwned::new(Attrs::new())).into_iter().map(|l| l.into_iter().map(|(t, _)| t).collect::<String>()).collect::<Vec<_>>().join("\n"),
                            }
                        };

                        info!("Server: {}", get_text(server_field));
                        info!("Username: {}", get_text(username_field));
                        next_state.set(LoginState::Connecting);
                        commands.insert_resource(LoginName(
                            get_text(username_field),
                        ));
                        commands.insert_resource(LoginServer(
                            get_text(server_field),
                        ));
                    },
                );
        })
        // .observe(
        //     move |mut trigger: Trigger<FocusedInput<KeyboardInput>>,
        //           q: Query<&TextEdit>,
        //           mut next_state: ResMut<NextState<LoginState>>,
        //           mut commands: Commands| {
        //         let event = &trigger.event().input;
        //         if !event.state.is_pressed() || event.logical_key != Key::Enter {
        //             return;
        //         }
        //         trigger.propagate(false);
        //         info!("Clicked button!");
        //         info!("Server: {}", q.get(server_field).unwrap().text());
        //         info!("Username: {}", q.get(username_field).unwrap().text());
        //         next_state.set(LoginState::Connecting);
        //         commands.insert_resource(LoginName(q.get(username_field).unwrap().text().into()));
        //         commands.insert_resource(LoginServer(q.get(server_field).unwrap().text().into()));
        //     },
        // );
        ;

    commands.insert_resource(FocusedWidget(Some(server_field)));
}

fn wait_for_connection_event(
    event_reader: NonSend<ConnectionEventChannel>,
    mut commands: Commands,
) {
    match event_reader.0.try_recv() {
        Ok(ev) => match ev {
            ConnectionEvent::ConnectionSuccessful(conn, player_id) => {
                commands.insert_resource(MyPlayerId(player_id));
                commands.insert_resource(LobbyConnection(conn));
                commands.trigger(ConnectionSuccessful)
            }
            ConnectionEvent::ConnectionFailed => commands.trigger(ConnectionFailed),
        },
        Err(TryRecvError::Empty) => {}
        Err(_) => todo!(),
    }
}

fn on_successful_connection(
    _: Trigger<ConnectionSuccessful>,
    mut next_state: ResMut<NextState<crate::State>>,
) {
    next_state.set(crate::State::Lobby);
}

fn on_failed_connection(
    _: Trigger<ConnectionFailed>,
    mut next_state: ResMut<NextState<LoginState>>,
    mut commands: Commands,
) {
    next_state.set(LoginState::Login);
    commands.info("Connection failed");
}

fn setup_connecting(
    server: Res<LoginServer>,
    username: Res<LoginName>,
    runtime: Res<TokioTasksRuntime>,
    mut commands: Commands,
) {
    build_ui_root(LoginState::Connecting, &mut commands).with_children(|parent| {
        parent.spawn(Text::new("Connecting..."));
        parent.spawn((Text::new("Cancel"), Button)).observe(
            |mut trigger: Trigger<Pointer<Click>>,
             mut next_state: ResMut<NextState<LoginState>>| {
                trigger.propagate(false);
                next_state.set(LoginState::Login);
            },
        );
    });

    let server = server.0.clone();
    let username = username.0.clone();

    let default_port = 54765;

    let last_colon_pos = server
        .char_indices()
        .filter(|&(i, c)| (c == ':'))
        .map(|(i, c)| i)
        .last();
    let (url, port) = if let Some(last_colon_pos) = last_colon_pos {
        let (url, possible_port) = server.split_at(last_colon_pos);
        if let Ok(port) = possible_port.parse() {
            (url, port)
        } else {
            (server.as_str(), default_port)
        }
    } else {
        (server.as_str(), default_port)
    };
    let server = format!("https://{url}:{port}");

    let (send, recv) = std::sync::mpsc::channel();

    commands.queue(move |world: &mut World| {
        world.insert_non_send_resource(ConnectionEventChannel(recv));
    });

    runtime.spawn_background_task(|_ctx| async move {
        match tokio::time::timeout(Duration::from_secs(30), try_connect(server, username))
            .await
            .flatten2()
        {
            Ok((conn, player_id)) => {
                info!("Connected!");
                let _ = send.send(ConnectionEvent::ConnectionSuccessful(conn, player_id));
            }
            Err(e) => {
                info!("Connection failed: {e}");
                let _ = send.send(ConnectionEvent::ConnectionFailed);
            }
        }
    });
}

async fn try_connect(addr: String, name: String) -> anyhow::Result<(Connection, PlayerId)> {
    println!("Building endpoint");
    let client = Endpoint::client(
        ClientConfig::builder()
            .with_bind_address_v6(
                SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0),
                Ipv6DualStackConfig::Allow,
            )
            .with_no_cert_validation()
            .build(),
    )?;
    println!("Connecting...");
    let conn = client.connect(addr).await?;
    println!("Connected...");
    println!("Sending handshake...");
    // Initiate handshake
    conn.open_uni()
        .await?
        .await?
        .write_message(MessageFromPlayer::InitialHandshake { name })
        .await?;
    println!("Waiting for id...");
    let id = conn.accept_uni().await?.read_message().await?;
    let id = match id {
        MessageFromServer::InitialHandshakeResponse { id } => id,
        _ => anyhow::bail!("Received invalid response from handshake"),
    };
    Ok((conn, id))
}
