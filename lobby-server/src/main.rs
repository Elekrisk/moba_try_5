#![feature(decl_macro)]
#![feature(try_blocks)]
#![feature(never_type)]
#![feature(never_type_fallback)]
#![feature(new_range_api)]
#![feature(async_closure)]

use core::range::{Range, RangeInclusive};
use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    net::{Ipv6Addr, SocketAddrV6},
    path::PathBuf,
    process::{Command, ExitStatus},
    sync::{Arc, Once},
    time::Duration,
};

use clap::Parser;
use lobby_server::{
    ChampSelectState, ChampionSelection, Lobby, LobbyId, LobbySettings, LobbyShortInfo, LobbyState,
    MessageFromGameServerToLobby, MessageFromLobbyToGameServer, MessageFromPlayer,
    MessageFromServer, PlayerId, PlayerInfo, PlayerSelection, ReadMessage as _, Team,
    WriteMessage as _,
};
use regex::{Regex, RegexBuilder};
use tokio::task::{JoinHandle, JoinSet};
use uuid::Uuid;
use wtransport::{
    config::Ipv6DualStackConfig, ClientConfig, Connection, Endpoint, Identity, ServerConfig,
};

#[derive(clap::Parser)]
struct Options {
    game_server_launch_mode: GameServerLaunchMode,
    game_server_path: PathBuf,
    #[arg(value_parser = parse_port_range)]
    game_server_port_range: RangeInclusive<u16>,
}

fn parse_port_range(arg: &str) -> anyhow::Result<RangeInclusive<u16>> {
    let regex = Regex::new(r"^(?:(?<single>\d+)|(?<start>\d+)-(?<end>\d+))$").unwrap();
    let captures = regex
        .captures(arg)
        .ok_or(anyhow::anyhow!("Invalid port range"))?;
    if let Some(single) = captures.name("single") {
        let port = single.as_str().parse()?;
        Ok(RangeInclusive {
            start: port,
            end: port,
        })
    } else {
        let start = captures
            .name("start")
            .ok_or(anyhow::anyhow!("Invalid port range"))?
            .as_str()
            .parse()?;
        let end = captures
            .name("end")
            .ok_or(anyhow::anyhow!("Invalid port range"))?
            .as_str()
            .parse()?;
        Ok(RangeInclusive { start, end })
    }
}

#[derive(Clone, clap::ValueEnum)]
enum GameServerLaunchMode {
    Executable,
    Cargo,
}

#[tokio::main]
async fn main() {
    let options = Options::parse();

    ServerState::new(options).run().await;
}

const MAPS: [MapDef; 1] = [MapDef {
    name: "Default",
    min_teams: 2,
    max_teams: 2,
}];

struct MapDef {
    name: &'static str,
    min_teams: usize,
    max_teams: usize,
}

// #[derive(Debug)]
enum Event {
    ConnectionMade(Connection),
    PlayerNameUpdated(PlayerId, String),
    MessageReceived(PlayerId, MessageFromPlayer),
    ConnectionLost(PlayerId),
    Callback(Box<dyn FnOnce(&mut ServerState) + Send + Sync + 'static>),
    Shutdown,
}

struct ServerState {
    options: Options,
    used_game_server_ports: HashSet<u16>,
    lobbies: HashMap<LobbyId, Lobby>,
    game_servers: HashMap<LobbyId, tokio::sync::oneshot::Sender<()>>,
    players: HashMap<PlayerId, PlayerInfoWithConn>,
    event_receiver: tokio::sync::mpsc::UnboundedReceiver<Event>,
    event_sender: tokio::sync::mpsc::UnboundedSender<Event>,
    should_exit: bool,
}

struct PlayerInfoWithConn {
    player: PlayerInfo,
    in_lobby: Option<LobbyId>,
    conn: Connection,
}

impl ServerState {
    fn new(options: Options) -> Self {
        let (event_sender, event_receiver) = tokio::sync::mpsc::unbounded_channel();
        Self {
            options,
            used_game_server_ports: HashSet::new(),
            lobbies: HashMap::new(),
            game_servers: HashMap::new(),
            players: HashMap::new(),
            event_sender,
            event_receiver,
            should_exit: false,
        }
    }

    async fn run(&mut self) {
        println!(
            "{} concurrent game servers supported",
            self.options.game_server_port_range.iter().count()
        );

        // Add ctrl-c handler

        let send = self.event_sender.clone();
        let mut has_called_handler = false;
        ctrlc::set_handler(move || {
            if !has_called_handler {
                println!("Ctrl-C pressed, shutting down...");
                has_called_handler = true;
                println!("Broadcasting to all connected clients that we are shutting down");
                let _ = send.send(Event::Shutdown);
            } else {
                println!("Ctrl-C pressed a second time, immediately shutting down");
                std::process::exit(130);
            }
        })
        .unwrap();

        // Start listening server
        let server = Endpoint::server(
            ServerConfig::builder()
                .with_bind_address_v6(
                    SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 54765, 0, 0),
                    Ipv6DualStackConfig::Allow,
                )
                .with_identity(Identity::self_signed(["localhost", "127.0.0.1", "::1"]).unwrap())
                .keep_alive_interval(Some(Duration::from_secs(15)))
                .build(),
        )
        .unwrap();

        let mut accept = Box::pin(server.accept());

        while !self.should_exit {
            let send = self.event_sender.clone();

            tokio::select! {
                session = &mut accept => {
                    println!("Connection received!");
                    accept = Box::pin(server.accept());
                    tokio::spawn(async move {
                        match session.await {
                            Ok(x) => match x.accept().await {
                                Ok(x) => { let _ = send.send(Event::ConnectionMade(x)); },
                                Err(e) => println!("Session request not accepted: {e}"),
                            },
                            Err(e) => println!("Session not accepted: {e}"),
                        }
                    });
                },
                msg = self.event_receiver.recv() => {
                    let Some(msg) = msg else { break };
                    // This should only ever block if we are shutting down,
                    // all other async activity is spawned in tasks
                    self.handle_event(msg).await;
                }
            };
        }
    }

    async fn handle_event(&mut self, msg: Event) {
        // println!("Event received: {msg:?}");
        match msg {
            Event::ConnectionMade(connection) => {
                let player_id = PlayerId::new();
                let conn = connection.clone();
                self.players.insert(
                    player_id,
                    PlayerInfoWithConn {
                        player: PlayerInfo {
                            id: player_id,
                            name: String::new(),
                        },
                        in_lobby: None,
                        conn,
                    },
                );

                let send = self.event_sender.clone();

                tokio::spawn(async move {
                    let x: anyhow::Result<()> = try {
                        let mut recv_stream = connection.accept_uni().await?;
                        let msg = recv_stream.read_message().await?;
                        let MessageFromPlayer::InitialHandshake { name } = msg else {
                            Err(anyhow::anyhow!("Wrong message received"))?;
                            unreachable!();
                        };

                        let _ = send.send(Event::PlayerNameUpdated(player_id, name));

                        connection
                            .open_uni()
                            .await?
                            .await?
                            .write_message(MessageFromServer::InitialHandshakeResponse {
                                id: player_id,
                            })
                            .await?;

                        let send = send.clone();
                        tokio::spawn(async move {
                            let Err(e): anyhow::Result<!> = try {
                                loop {
                                    let mut recv_stream = connection.accept_uni().await?;
                                    let msg = recv_stream.read_message().await?;

                                    if send.send(Event::MessageReceived(player_id, msg)).is_err() {
                                        return;
                                    }
                                }
                            };
                            println!("Error (1): {e}");
                            let _ = send.send(Event::ConnectionLost(player_id));
                        });
                    };

                    if let Err(e) = x {
                        println!("Error (2): {e}");
                        let _ = send.send(Event::ConnectionLost(player_id));
                    }
                });
            }
            Event::PlayerNameUpdated(player_id, name) => {
                if let Some(player) = self.players.get_mut(&player_id) {
                    player.player.name = name;
                }
            }
            Event::MessageReceived(player_id, msg) => {
                self.handle_message(player_id, msg);
            }
            Event::ConnectionLost(player_id) => {
                // We need to handle removing the player from the lobby it is in, if any.
                self.handle_player_left_lobby(player_id);
                self.players.remove(&player_id);
            }
            Event::Callback(func) => {
                func(self);
            }
            Event::Shutdown => {
                let handles = self.broadcast_global_message(MessageFromServer::ServerShutdown);
                self.should_exit = true;
                JoinSet::from_iter(handles).join_all().await;
            }
        }
    }

    fn handle_message(&mut self, player_id: PlayerId, msg: MessageFromPlayer) {
        println!("Message received from {player_id:?}: {msg:?}");
        let Some(player) = self.players.get_mut(&player_id) else {
            return;
        };

        macro_rules! guards {
            (ret $e:expr) => {
                {
                    self.send_message(player_id, MessageFromServer::RequestRefused($e.into()));
                    return;
                }
            };
            ($([$($tt:tt)*])*) => {
                $(guards!($($tt)*);)*
            };
            (Ok($pat:pat) = $guard:expr) => {
                let $pat = match $guard {
                    Ok(val) => val,
                    Err(e) => guards!(ret e),
                };
            };
            (Ok($pat:pat) = $guard:expr => $msg:expr) => {
                let $pat = match $guard {
                    Ok(val) => val,
                    Err(_) => guards!(ret $msg),
                };
            };
            (Some($pat:pat) = $guard:expr => $msg:expr) => {
                let $pat = match $guard {
                    Some(val) => val,
                    None => guards!(ret $msg),
                };
            };
            ($guard:expr => $msg:expr) => {
                if $guard { guards!(ret $msg) }
            };
            ($guard:expr) => {
                if let Err(e) = $guard { guards!(ret e) }
            };
        }

        macro_rules! not_in_lobby {
            () => {
                match player.in_lobby {
                    Some(_) => Err("You are already in a lobby."),
                    None => Ok(()),
                }
            };
        }

        macro_rules! in_lobby {
            () => {
                player.in_lobby.ok_or("You are not in a lobby.")
            };
        }

        macro_rules! lobby_exists {
            ($lobby_id:expr) => {
                self.lobbies
                    .get_mut(&$lobby_id)
                    .ok_or("That lobby does not exist.")
            };
        }

        macro_rules! normal_lobby {
            ($lobby:expr) => {
                if matches!($lobby.lobby_state, LobbyState::Normal) {
                    Ok(())
                } else {
                    Err("Lobby is in invalid state.")
                }
            };
        }

        macro_rules! champ_select {
            ($lobby:expr) => {
                if let LobbyState::ChampSelect(state) = &mut $lobby.lobby_state {
                    Ok(state)
                } else {
                    Err("Lobby is in invalid state.")
                }
            };
        }

        // let lobby_exists = |lobby_id| {
        //     let Some(lobby) = self.lobbies.get_mut(&lobby_id) else {
        //         self.send_message(
        //             player_id,
        //             MessageFromServer::RequestRefused("That lobby does not exist.".into()),
        //         );
        //         return;
        //     };
        // };

        match msg {
            MessageFromPlayer::InitialHandshake { .. } => {}
            MessageFromPlayer::CreateLobby => {
                guards! {
                    [not_in_lobby!()]
                }

                let lobby_id = LobbyId::new();
                let lobby = Lobby {
                    id: lobby_id,
                    settings: LobbySettings {
                        name: format!("{}'s Lobby", player.player.name),
                        map: "Default".into(),
                        team_count: 2,
                        player_limit_per_team: 5,
                        players_can_change_team: true,
                        lobby_is_open: true,
                    },
                    leader: player_id,
                    players: [(Team(0), vec![player_id]), (Team(1), vec![])].into(),
                    lobby_state: LobbyState::Normal,
                };

                self.lobbies.insert(lobby_id, lobby);
                player.in_lobby = Some(lobby_id);

                self.send_message(player_id, MessageFromServer::YouJoinedLobby(lobby_id));
                self.broadcast_lobby_message(
                    lobby_id,
                    Some(player_id),
                    MessageFromServer::PlayerJoinedYourLobby(player_id),
                );
            }
            MessageFromPlayer::JoinLobby(lobby_id) => {
                guards! {
                    [not_in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [normal_lobby!(lobby)]
                    [!lobby.settings.lobby_is_open => "The lobby is closed."]
                    [lobby.players.values().map(Vec::len).sum::<usize>() >= lobby.settings.team_count * lobby.settings.player_limit_per_team => "The lobby is full"]
                }

                // Find which team to join
                // We want to join the team with the fewest players

                let team_player_count = (0..lobby.settings.team_count)
                    .map(|i| (Team(i), lobby.players.get(&Team(i)).unwrap().len()))
                    .min_by_key(|x| x.1)
                    .expect("There should always be at least 1 team");

                player.in_lobby = Some(lobby_id);

                lobby
                    .players
                    .get_mut(&team_player_count.0)
                    .unwrap()
                    .push(player_id);
                self.send_message(player_id, MessageFromServer::YouJoinedLobby(lobby_id));
                self.broadcast_lobby_message(
                    lobby_id,
                    Some(player_id),
                    MessageFromServer::PlayerJoinedYourLobby(player_id),
                );
            }
            MessageFromPlayer::LeaveLobby => {
                self.send_message(player_id, MessageFromServer::YouLeftLobby);
                self.handle_player_left_lobby(player_id);
            }
            MessageFromPlayer::SwitchTeam(id, team) => {
                guards! {
                    [Ok(lobby_id) = in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [normal_lobby!(lobby)]
                    [!lobby.settings.players_can_change_team && lobby.leader != player_id => "Team switching is disabled in this lobby."]
                    [id != player_id && lobby.leader != player_id => "Cannot switch team of other player."]
                    [!lobby.players.contains_key(&team) => format!("{team} does not exist.")]
                    [lobby.players.get(&team).unwrap().len() >= lobby.settings.player_limit_per_team => format!("{team} is full.")]
                }

                for players in lobby.players.values_mut() {
                    if let Some(pos) = players.iter().position(|p| *p == id) {
                        players.remove(pos);
                        break;
                    }
                }

                lobby.players.get_mut(&team).unwrap().push(id);

                self.broadcast_lobby_message(
                    lobby_id,
                    None,
                    MessageFromServer::PlayerSwitchedTeam(id, team),
                );
            }
            MessageFromPlayer::GetLobbyInfo(lobby_id) => {
                guards!(Ok(lobby) = lobby_exists!(lobby_id));

                let message = MessageFromServer::LobbyInfo(lobby.clone());
                self.send_message(player_id, message);
            }
            MessageFromPlayer::GetLobbyList => {
                let list = self
                    .lobbies
                    .values()
                    .map(|lobby| LobbyShortInfo {
                        id: lobby.id,
                        name: lobby.settings.name.clone(),
                        player_count: lobby.players.values().map(Vec::len).sum(),
                        max_player_count: lobby.settings.team_count
                            * lobby.settings.player_limit_per_team,
                    })
                    .collect();
                self.send_message(player_id, MessageFromServer::LobbyList(list));
            }
            MessageFromPlayer::GetPlayerInfo(id) => {
                match self.players.get(&id) {
                    Some(player) => {
                        self.send_message(
                            player_id,
                            MessageFromServer::PlayerInfo(player.player.clone()),
                        );
                    }
                    None => {
                        // TODO: figure out what's supposed to be done here
                    }
                }
            }
            MessageFromPlayer::Disconnecting => {
                let _ = self.event_sender.send(Event::ConnectionLost(player_id));
            }
            MessageFromPlayer::KickPlayer(id) => {
                guards! {
                    [Ok(lobby_id) = in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [normal_lobby!(lobby)]
                    [lobby.leader != player_id => "You are not the lobby leader."]
                }

                self.send_message(id, MessageFromServer::YouLeftLobby);
                self.handle_player_left_lobby(id);
            }
            MessageFromPlayer::UpdateSettings(lobby_settings) => {
                guards! {
                    [Ok(lobby_id) = in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [normal_lobby!(lobby)]
                    [lobby.leader != player_id => "You are not the lobby leader."]
                    [lobby_settings.name.is_empty() => "Lobby name cannot be empty."]
                    [lobby_settings.name.chars().all(char::is_whitespace) => "Lobby name cannot be only whitespace."]
                    [Some(map) = MAPS.iter().find(|map| map.name == lobby_settings.map) => format!("No map {:?} exists.", lobby_settings.map)]
                    [lobby_settings.team_count < 1 => "There must be at least 1 team."]
                    // [!(map.min_teams..=map.max_teams).contains(&lobby_settings.team_count) => format!("Map {:?} doesn't support {} teams;\nmust be between {} and {}", map.name, lobby_settings.team_count, map.min_teams, map.max_teams)]
                }

                if lobby_settings == lobby.settings {
                    return;
                }

                let mut players_to_reshuffle = vec![];

                match lobby_settings.team_count.cmp(&lobby.settings.team_count) {
                    Ordering::Less => {
                        for team in (lobby_settings.team_count..lobby.settings.team_count).map(Team)
                        {
                            players_to_reshuffle.append(&mut lobby.players.remove(&team).unwrap());
                        }
                    }
                    Ordering::Greater => {
                        for team in (lobby.settings.team_count..lobby_settings.team_count).map(Team)
                        {
                            lobby.players.insert(team, vec![]);
                        }
                    }
                    _ => {}
                }

                if lobby_settings.player_limit_per_team < lobby.settings.player_limit_per_team
                    || lobby
                        .players
                        .values()
                        .any(|v| v.len() > lobby_settings.player_limit_per_team)
                {
                    for players in lobby.players.values_mut() {
                        if players.len() > lobby_settings.player_limit_per_team {
                            players_to_reshuffle
                                .extend(players.drain(lobby_settings.player_limit_per_team..));
                        }
                    }
                }

                for player in players_to_reshuffle {
                    let team_player_count = (0..lobby_settings.team_count)
                        .map(|i| (Team(i), lobby.players.get(&Team(i)).unwrap().len()))
                        .min_by_key(|x| x.1)
                        .expect("There should always be at least 1 teams");

                    lobby
                        .players
                        .get_mut(&team_player_count.0)
                        .unwrap()
                        .push(player);
                }

                lobby.settings = lobby_settings.clone();
                self.broadcast_lobby_message(
                    lobby_id,
                    None,
                    MessageFromServer::SettingsUpdated(lobby_settings),
                );
            }
            MessageFromPlayer::SwitchPlaces(player_a, player_b) => {
                guards! {
                    [Ok(lobby_id) = in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [normal_lobby!(lobby)]
                    [!lobby.settings.players_can_change_team && lobby.leader != player_id => "Team switching is disabled in this lobby."]
                    [lobby.leader != player_id => "Non-leader cannot switch places of players."]
                }

                let Some(pos_a) = lobby
                    .players
                    .iter()
                    .find_map(|(t, v)| v.iter().position(|p| *p == player_a).map(|i| (*t, i)))
                else {
                    self.send_message(
                        player_id,
                        MessageFromServer::RequestRefused("Player does not exist".into()),
                    );
                    return;
                };
                let Some(pos_b) = lobby
                    .players
                    .iter()
                    .find_map(|(t, v)| v.iter().position(|p| *p == player_b).map(|i| (*t, i)))
                else {
                    self.send_message(
                        player_id,
                        MessageFromServer::RequestRefused("Player does not exist".into()),
                    );
                    return;
                };

                lobby.players.get_mut(&pos_a.0).unwrap()[pos_a.1] = player_b;
                lobby.players.get_mut(&pos_b.0).unwrap()[pos_b.1] = player_a;

                self.broadcast_lobby_message(
                    lobby_id,
                    None,
                    MessageFromServer::PlayersSwitched(player_a, player_b),
                );
            }
            MessageFromPlayer::EnterChampSelect => {
                guards! {
                    [Ok(lobby_id) = in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [normal_lobby!(lobby)]
                    [lobby.leader != player_id => "Non-leader cannot trigger champ select."]
                }

                let new_state = LobbyState::ChampSelect(ChampSelectState {
                    available_champs: (1..=100).map(|d| format!("Champ {d}")).collect(),
                    selected_champs: lobby
                        .players
                        .values()
                        .flatten()
                        .map(|p| (*p, None))
                        .collect(),
                });

                lobby.lobby_state = new_state;

                self.broadcast_lobby_message(lobby_id, None, MessageFromServer::ChampSelectEntered);
            }
            MessageFromPlayer::SelectChampion(champion) => {
                guards! {
                    [Ok(lobby_id) = in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [Ok(state) = champ_select!(lobby)]
                    [!state.available_champs.contains(&champion) => "That champion does not exist."]
                    [state.selected_champs.get(&player_id).unwrap().as_ref().map(|x| x.locked).unwrap_or(false) => "You cannot change locked selection."]
                }

                state.selected_champs.insert(
                    player_id,
                    Some(ChampionSelection {
                        champion: champion.clone(),
                        locked: false,
                    }),
                );
                self.broadcast_lobby_message(
                    lobby_id,
                    None,
                    MessageFromServer::PlayerSelectedChampion(player_id, champion),
                );
            }
            MessageFromPlayer::LockChampSelection => {
                guards! {
                    [Ok(lobby_id) = in_lobby!()]
                    [Ok(lobby) = lobby_exists!(lobby_id)]
                    [Ok(state) = champ_select!(lobby)]
                    [!state.selected_champs.get(&player_id).unwrap().is_some() => "Cannot lock empty selection."]
                }

                state
                    .selected_champs
                    .get_mut(&player_id)
                    .unwrap()
                    .as_mut()
                    .unwrap()
                    .locked = true;

                if state
                    .selected_champs
                    .values()
                    .all(|s| s.as_ref().is_some_and(|s| s.locked))
                {
                    // All players locked: start game
                    self.start_game(lobby_id);
                }

                self.broadcast_lobby_message(
                    lobby_id,
                    None,
                    MessageFromServer::ChampSelectionLocked(player_id),
                );
            }
            MessageFromPlayer::StartGame => todo!(),
        }
    }

    fn handle_player_left_lobby(&mut self, player_id: PlayerId) {
        let Some(player) = self.players.get_mut(&player_id) else {
            return;
        };
        let Some(lobby_id) = player.in_lobby else {
            return;
        };
        let Some(lobby) = self.lobbies.get_mut(&lobby_id) else {
            return;
        };

        // Remove player from lobby
        for players in lobby.players.values_mut() {
            if let Some(pos) = players.iter().position(|p| *p == player_id) {
                players.remove(pos);
                break;
            }
        }
        player.in_lobby = None;

        // If that player was the last player, delete the lobby
        if lobby.players.values().all(Vec::is_empty) {
            self.lobbies.remove(&lobby_id);

            // If a game server is running for this lobby, kill it
            if let Some(kill) = self.game_servers.remove(&lobby_id) {
                let _ = kill.send(());
            }
            return;
        }

        // If that player was the leader, we need to select a new one
        if lobby.leader == player_id {
            // We don't really care who, so we choose the first one in the list
            lobby.leader = *lobby.players.values().flatten().next().unwrap();
            let message = MessageFromServer::LobbyLeaderChanged(lobby.leader);
            self.broadcast_lobby_message(lobby_id, None, message);
        }

        self.broadcast_lobby_message(
            lobby_id,
            None,
            MessageFromServer::PlayerLeftYourLobby(player_id),
        );
    }

    fn start_game(&mut self, lobby_id: LobbyId) {
        let Some(lobby) = self.lobbies.get(&lobby_id) else {
            todo!();
        };

        let LobbyState::ChampSelect(selections) = &lobby.lobby_state else {
            todo!();
        };

        if selections.selected_champs.iter().any(|s| s.1.is_none()) {
            todo!();
        }

        // find free port

        let Some(port) = self
            .options
            .game_server_port_range
            .into_iter()
            .find(|port| !self.used_game_server_ports.contains(port))
        else {
            todo!()
        };

        // Start game server

        let (send, mut recv) = tokio::sync::oneshot::channel();
        self.game_servers.insert(lobby_id, send);

        let lobby_token = Uuid::new_v4();

        let mut cmdline = vec![];
        match self.options.game_server_launch_mode {
            GameServerLaunchMode::Executable => {
                cmdline.push(self.options.game_server_path.to_string_lossy().to_string());
            }
            GameServerLaunchMode::Cargo => {
                cmdline.extend(
                    "cargo run --bin=server --"
                        .split_whitespace()
                        .map(String::from),
                );
            }
        }
        cmdline.push(lobby_token.to_string());
        cmdline.push(port.to_string());

        let dir = if self.options.game_server_path.is_dir() {
            self.options.game_server_path.as_path()
        } else {
            self.options.game_server_path.parent().unwrap()
        };

        let Ok(mut process) = tokio::process::Command::new(&cmdline[0])
            .args(&cmdline[1..])
            .current_dir(dir)
            .spawn()
        else {
            self.broadcast_lobby_message(
                lobby_id,
                None,
                MessageFromServer::RequestRefused(
                    "Failed to start game server;\nplease restart your game client.".into(),
                ),
            );
            return;
        };

        let players = lobby
            .players
            .iter()
            .map(|(k, v)| {
                (
                    *k,
                    v.iter()
                        .map(|p| PlayerSelection {
                            player: self.players.get(p).unwrap().player.clone(),
                            champion: selections
                                .selected_champs
                                .get(p)
                                .unwrap()
                                .as_ref()
                                .unwrap()
                                .champion
                                .clone(),
                        })
                        .collect(),
                )
            })
            .collect();

        let connect_task = async move {
            let x: anyhow::Result<MessageFromGameServerToLobby> = try {
                let client = Endpoint::client(
                    ClientConfig::builder()
                        .with_bind_address_v6(
                            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0),
                            Ipv6DualStackConfig::Allow,
                        )
                        .with_no_cert_validation()
                        .build(),
                )?;
                println!("Lobby server trying to connect to game server...");
                let conn = client.connect(format!("https://localhost:{port}")).await?;
                println!("Lobby server connected! Writing message...");
                conn.open_uni()
                    .await?
                    .await?
                    .write_message(MessageFromLobbyToGameServer::LobbyInitialMessage {
                        token: lobby_token,
                        players,
                    })
                    .await?;
                println!("Message written! Reading message...");
                conn.accept_uni().await?.read_message_framed().await?
            };
            println!("Message read!");
            x
        };

        let s = self.event_sender.clone();

        tokio::spawn(async move {
            let wait_for_exit =
                async move |recv: &mut tokio::sync::oneshot::Receiver<()>,
                            process: &mut tokio::process::Child| {
                    tokio::select! {
                        _ = recv => {
                            eprintln!("Killing server");
                            process.kill().await.unwrap();
                            process.wait().await
                        }
                        exit = process.wait() => exit
                    }
                };

            let on_exit = |exit: std::io::Result<ExitStatus>| {
                println!("Game server exited");
                if exit.is_ok_and(|c| c.success()) {
                    s.send(Event::Callback(Box::new(move |s| {
                        let Some(lobby) = s.lobbies.get_mut(&lobby_id) else {
                            return;
                        };
                        let players: Vec<_> = lobby.players.values().flatten().copied().collect();
                        for player in players {
                            s.send_message(player, MessageFromServer::YouLeftLobby);
                            s.handle_player_left_lobby(player);
                        }
                    })))
                    .unwrap();
                } else {
                    // Failed!
                    s.send(Event::Callback(Box::new(move |s| {
                        s.broadcast_lobby_message(
                            lobby_id,
                            None,
                            MessageFromServer::RequestRefused("Failed to start game server".into()),
                        );
                        let Some(lobby) = s.lobbies.get_mut(&lobby_id) else {
                            eprintln!("????????");
                            return;
                        };
                        let players: Vec<_> = lobby.players.values().flatten().copied().collect();
                        for player in players {
                            s.send_message(player, MessageFromServer::YouLeftLobby);
                            s.handle_player_left_lobby(player);
                        }
                    })))
                    .unwrap();
                }
                s.send(Event::Callback(Box::new(move |s| {
                    s.game_servers.remove(&lobby_id);
                })))
                .unwrap();
            };

            let x = tokio::select! {
                exit = wait_for_exit(&mut recv, &mut process) => {
                    on_exit(exit);
                    return;
                }
                x = connect_task => {
                    x
                }
            };

            match x {
                Ok(x) => {
                    // Success!
                    let MessageFromGameServerToLobby::PlayerTokensGenerated { players } = x;
                    s.send(Event::Callback(Box::new(move |s| {
                        for (player, token) in players {
                            s.send_message(player, MessageFromServer::GameStarted(token))
                                .unwrap();
                        }
                    })))
                    .unwrap();
                }
                Err(e) => {
                    // Error!
                    eprintln!("Error: {e}");
                    process.kill().await.unwrap();
                }
            }
            on_exit(wait_for_exit(&mut recv, &mut process).await);
        });
    }

    fn send_message(
        &mut self,
        player_id: PlayerId,
        message: MessageFromServer,
    ) -> Option<JoinHandle<()>> {
        let conn = self.players.get(&player_id).map(|p| p.conn.clone())?;
        Some(tokio::spawn(async move {
            let _: anyhow::Result<()> = try {
                conn.open_uni().await?.await?.write_message(message).await?;
            };
        }))
    }

    fn broadcast_lobby_message(
        &mut self,
        lobby_id: LobbyId,
        exclude_player: Option<PlayerId>,
        message: MessageFromServer,
    ) -> Vec<JoinHandle<()>> {
        let Some(lobby) = self.lobbies.get(&lobby_id) else {
            return vec![];
        };
        let message = serde_json::to_vec_pretty(&message).unwrap();
        let message: Arc<[u8]> = message.into();
        lobby
            .players
            .values()
            .flatten()
            .filter_map(|player| {
                if Some(*player) == exclude_player {
                    return None;
                }
                let conn = self.players.get(player).map(|p| p.conn.clone())?;
                let message = message.clone();
                Some(tokio::spawn(async move {
                    let _: anyhow::Result<()> = try {
                        conn.open_uni()
                            .await?
                            .await?
                            .write_message_raw(&message)
                            .await?;
                    };
                }))
            })
            .collect()
    }

    fn broadcast_global_message(&mut self, message: MessageFromServer) -> Vec<JoinHandle<()>> {
        let message = serde_json::to_vec_pretty(&message).unwrap();
        let message: Arc<[u8]> = message.into();
        self.players
            .values()
            .map(|player| {
                let conn = player.conn.clone();
                let message = message.clone();
                tokio::spawn(async move {
                    let _: anyhow::Result<()> = try {
                        conn.open_uni()
                            .await?
                            .await?
                            .write_message_raw(&message)
                            .await?;
                    };
                })
            })
            .collect()
    }
}

// trait ReadMessage {
//     async fn read_message(&mut self) -> anyhow::Result<MessageFromPlayer>;
// }

// impl ReadMessage for RecvStream {
//     async fn read_message(&mut self) -> anyhow::Result<MessageFromPlayer> {
//         let mut buf = vec![];
//         self.read_to_end(&mut buf).await?;
//         let msg = serde_json::from_slice(&buf)?;
//         Ok(msg)
//     }
// }

// trait WriteMessage {
//     async fn write_message(&mut self, msg: MessageFromServer) -> anyhow::Result<()>;
//     async fn write_message_raw(&mut self, msg: &[u8]) -> anyhow::Result<()>;
// }

// impl WriteMessage for SendStream {
//     async fn write_message(&mut self, msg: MessageFromServer) -> anyhow::Result<()> {
//         self.write_all(&serde_json::to_vec_pretty(&msg)?).await?;
//         Ok(())
//     }
//     async fn write_message_raw(&mut self, msg: &[u8]) -> anyhow::Result<()> {
//         self.write_all(msg).await?;
//         Ok(())
//     }
// }
