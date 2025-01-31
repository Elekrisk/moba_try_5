use std::{collections::HashMap, fmt::Display};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt};
use uuid::Uuid;
use wtransport::{RecvStream, SendStream};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Team(pub usize);

impl Display for Team {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Self::RED => f.write_str("Red Team"),
            Self::BLUE => f.write_str("Blue Team"),
            Team(n) => write!(f, "Team {n}"),
        }
    }
}

impl Team {
    pub const RED: Self = Self(0);
    pub const BLUE: Self = Self(1);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LobbyId(Uuid);

impl LobbyId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lobby {
    pub id: LobbyId,
    pub settings: LobbySettings,
    pub leader: PlayerId,
    pub players: HashMap<Team, Vec<PlayerId>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbySettings {
    pub name: String,
    pub map: String,
    pub team_count: usize,
    pub player_limit_per_team: usize,
    pub players_can_change_team: bool,
    pub lobby_is_open: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LobbyShortInfo {
    pub id: LobbyId,
    pub name: String,
    pub player_count: usize,
    pub max_player_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(Uuid);

impl PlayerId {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInfo {
    pub id: PlayerId,
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum MessageFromPlayer {
    InitialHandshake { name: String },
    CreateLobby,
    JoinLobby(LobbyId),
    LeaveLobby,
    SwitchTeam(PlayerId, Team),
    SwitchPlaces(PlayerId, PlayerId),
    GetLobbyInfo(LobbyId),
    GetLobbyList,
    GetPlayerInfo(PlayerId),
    KickPlayer(PlayerId),
    UpdateSettings(LobbySettings),
    StartGame,
    Disconnecting,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum MessageFromServer {
    InitialHandshakeResponse { id: PlayerId },
    YouJoinedLobby(LobbyId),
    YouLeftLobby,
    PlayerJoinedYourLobby(PlayerId),
    PlayerLeftYourLobby(PlayerId),
    PlayerSwitchedTeam(PlayerId, Team),
    PlayersSwitched(PlayerId, PlayerId),
    LobbyInfo(Lobby),
    LobbyList(Vec<LobbyShortInfo>),
    PlayerInfo(PlayerInfo),
    LobbyLeaderChanged(PlayerId),
    RequestRefused(String),
    SettingsUpdated(LobbySettings),
    GameStarted(String),
    ServerShutdown,
}

pub trait ReadMessage {
    async fn read_message<T: for<'a> Deserialize<'a>>(&mut self) -> anyhow::Result<T>;
    async fn read_message_framed<T: for<'a> Deserialize<'a>>(&mut self) -> anyhow::Result<T>;
}

impl ReadMessage for RecvStream {
    async fn read_message<T: for<'a> Deserialize<'a>>(&mut self) -> anyhow::Result<T> {
        let mut buf = vec![];
        self.read_to_end(&mut buf).await?;
        let msg = serde_json::from_slice(&buf)?;
        Ok(msg)
    }
    async fn read_message_framed<T: for<'a> Deserialize<'a>>(&mut self) -> anyhow::Result<T> {
        let len = self.read_u32().await?;
        let mut buf = vec![0; len as _];
        self.read_exact(&mut buf).await?;
        let msg = serde_json::from_slice(&buf)?;
        Ok(msg)
    }
}

pub trait WriteMessage {
    async fn write_message<T: Serialize>(&mut self, msg: T) -> anyhow::Result<()>;
    async fn write_message_framed<T: Serialize>(&mut self, msg: T) -> anyhow::Result<()>;
    async fn write_message_raw(&mut self, msg: &[u8]) -> anyhow::Result<()>;
}

impl WriteMessage for SendStream {
    async fn write_message<T: Serialize>(&mut self, msg: T) -> anyhow::Result<()> {
        self.write_all(&serde_json::to_vec_pretty(&msg)?).await?;
        Ok(())
    }
    async fn write_message_framed<T: Serialize>(&mut self, msg: T) -> anyhow::Result<()> {
        let data = serde_json::to_vec_pretty(&msg)?;
        let len = data.len();
        self.write_u32(len.try_into()?).await?;
        self.write_all(&data).await?;
        Ok(())
    }
    async fn write_message_raw(&mut self, msg: &[u8]) -> anyhow::Result<()> {
        self.write_all(msg).await?;
        Ok(())
    }
}
