use bevy::prelude::*;
use lobby_server::{LobbyState, MessageFromPlayer, Team};

use crate::ui::{OnClickExt, ScrollEvent};

use super::{LobbyBuildingContext, SendMessage};

pub fn build_champ_select(ctx: &LobbyBuildingContext, parent: &mut ChildBuilder) {
    let LobbyState::ChampSelect(state) = &ctx.lobby.lobby_state else {
        unreachable!()
    };

    // We need three columns; Leftmost and rightmost for teams, and middle for champions
    // Left column contains teams 2n, right column contains 2n + 1, for n € N*

    // Column container
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            width: Val::Percent(100.0),
            max_height: Val::Percent(80.0),
            ..default()
        })
        .with_children(|parent| {
            let left_sync = parent
                .spawn(Node {
                    width: Val::Percent(100.0 / 3.0),
                    flex_direction: FlexDirection::Column,
                    overflow: Overflow::scroll_y(),
                    ..default()
                })
                .with_children(|parent| build_team_list(ctx, 0, parent))
                .id();
            parent
                .spawn(Node {
                    width: Val::Percent(100.0 / 3.0),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(10.0),
                    column_gap: Val::Px(10.0),
                    overflow: Overflow::scroll_y(),
                    ..default()
                })
                .with_children(|parent| build_champ_list(ctx, parent))
                .observe(super::scroll);
            let right_sync = parent
                .spawn(Node {
                    width: Val::Percent(100.0 / 3.0),
                    flex_direction: FlexDirection::Column,
                    overflow: Overflow::scroll_y(),
                    ..default()
                })
                .with_children(|parent| build_team_list(ctx, 1, parent))
                .id();

            let sync = move |mut trigger: Trigger<ScrollEvent>,
                             mut q: Query<&mut ScrollPosition>| {
                info!("Sync scroll!");
                let event = trigger.event();
                if let Ok(mut scroll) = q.get_mut(left_sync) {
                    scroll.offset_x -= event.dx;
                    scroll.offset_y -= event.dy;
                }
                if let Ok(mut scroll) = q.get_mut(right_sync) {
                    scroll.offset_x -= event.dx;
                    scroll.offset_y -= event.dy;
                }
                trigger.propagate(false);
            };

            let mut observer = Observer::new(sync);
            observer.watch_entity(left_sync);
            observer.watch_entity(right_sync);
            parent.spawn(observer);
        });
    // Bottom button bar
    parent
        .spawn(Node {
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            // position_type: PositionType::Absolute,
            // bottom: Val::Px(0.0),
            ..default()
        })
        .with_children(|parent| {
            parent
                .spawn((Button, Text::new("[Lock]")))
                .on_click(|send: Res<SendMessage>| {
                    let _ = send.send(MessageFromPlayer::LockChampSelection);
                });
        });
}

// Builds a team list for teams 2n + offset (offset must be 0 or 1)
pub fn build_team_list(ctx: &LobbyBuildingContext, offset: usize, parent: &mut ChildBuilder) {
    let LobbyState::ChampSelect(state) = &ctx.lobby.lobby_state else {
        unreachable!()
    };

    for team in (0..)
        .map(|x| x * 2 + offset)
        .take_while(|x| *x < ctx.lobby.settings.team_count)
        .map(Team)
    {
        // Team container
        parent
            .spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: if offset == 0 {
                    AlignItems::Start
                } else {
                    AlignItems::End
                },
                ..default()
            })
            .with_children(|parent| {
                // Team title
                parent.spawn(Text::new(team.to_string()));
                // List of players and their chosen champion
                for player in ctx.lobby.players.get(&team).unwrap() {
                    // Player entry container
                    parent
                        .spawn(Node {
                            column_gap: Val::Px(20.0),
                            ..default()
                        })
                        .with_children(|parent| {
                            // Player name
                            let name = &ctx.player_cache.players.get(player).unwrap().name;
                            parent.spawn(Text::new(name));
                            // If champ has been chosen, the chosen champ
                            if let Some(champ) = state.selected_champs.get(player).unwrap() {
                                let text = match champ.locked {
                                    true => format!("*{}*", champ.champion),
                                    false => champ.champion.clone(),
                                };
                                parent.spawn(Text::new(text));
                            }
                        });
                }
            });
    }
}

pub fn build_champ_list(ctx: &LobbyBuildingContext, parent: &mut ChildBuilder) {
    let LobbyState::ChampSelect(state) = &ctx.lobby.lobby_state else {
        unreachable!()
    };

    for champ in &state.available_champs {
        let champ_clone = champ.clone();
        parent
            .spawn((Button, Text::new(format!("[{champ}]"))))
            .on_click(move |send: Res<SendMessage>| {
                let _ = send.send(MessageFromPlayer::SelectChampion(champ_clone.clone()));
            });
    }
}
