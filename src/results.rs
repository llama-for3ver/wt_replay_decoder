use crate::parser::PlayerData;
use crate::parser::PlayerInfo;
use crate::parser::PlayerReplayData;
use crate::parser::ReplayResults;
use anyhow::Context;
use anyhow::Result;

pub fn parse_replay_results_json(json_data: &str) -> Result<ReplayResults> {
    let json_value: serde_json::Value =
        serde_json::from_str(json_data).context("Failed to parse JSON")?;

    let obj = json_value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Root JSON is not an object"))?;

    let status = obj
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let time_played = obj
        .get("timePlayed")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let author_user_id = obj
        .get("authorUserId")
        .and_then(|v| v.as_str())
        .unwrap_or("-1")
        .to_string();

    let author = obj
        .get("author")
        .and_then(|v| v.as_str())
        .unwrap_or("server")
        .to_string();

    let mut players = Vec::new();

    if let Some(player_array) = obj.get("player").and_then(|v| v.as_array()) {
        if let Some(ui_scripts_data) = obj.get("uiScriptsData").and_then(|v| v.as_object()) {
            if let Some(players_info) = ui_scripts_data
                .get("playersInfo")
                .and_then(|v| v.as_object())
            {
                for player_data in player_array {
                    if let Some(player_obj) = player_data.as_object() {
                        let user_id_str = player_obj
                            .get("userId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        let mut player_info = None;
                        for (_, info_value) in players_info {
                            if let Some(info_obj) = info_value.as_object() {
                                let info_id =
                                    info_obj.get("id").and_then(|v| v.as_u64()).unwrap_or(0);

                                if info_id.to_string() == user_id_str
                                    || user_id_str.parse::<u64>().unwrap_or(0) == info_id
                                {
                                    player_info = Some(PlayerInfo {
                                        user_id: user_id_str.clone(),
                                        username: info_obj
                                            .get("name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        squadron_id: info_obj
                                            .get("clanId")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        squadron_tag: info_obj
                                            .get("squadronTag")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        platform: info_obj
                                            .get("platform")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                    });
                                    break;
                                }
                            }
                        }

                        if let Some(info) = player_info {
                            let mut lineup = Vec::new();
                            for (_, info_value) in players_info {
                                if let Some(info_obj) = info_value.as_object() {
                                    let info_id =
                                        info_obj.get("id").and_then(|v| v.as_u64()).unwrap_or(0);

                                    if info_id.to_string() == user_id_str
                                        || user_id_str.parse::<u64>().unwrap_or(0) == info_id
                                    {
                                        if let Some(crafts) =
                                            info_obj.get("crafts").and_then(|v| v.as_object())
                                        {
                                            for (_, craft_name) in crafts {
                                                if let Some(name) = craft_name.as_str() {
                                                    lineup.push(name.to_string());
                                                }
                                            }
                                        }
                                        break;
                                    }
                                }
                            }

                            let replay_data = PlayerReplayData {
                                user_id: user_id_str.clone(),
                                squad: player_obj
                                    .get("squadId")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as i32,
                                auto_squad: player_obj
                                    .get("autoSquad")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false),
                                team: player_obj.get("team").and_then(|v| v.as_i64()).unwrap_or(0)
                                    as i32,
                                wait_time: {
                                    let mut wait_time = 0.0;
                                    for (_, info_value) in players_info {
                                        if let Some(info_obj) = info_value.as_object() {
                                            let info_id = info_obj
                                                .get("id")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(0);
                                            if info_id.to_string() == user_id_str
                                                || user_id_str.parse::<u64>().unwrap_or(0)
                                                    == info_id
                                            {
                                                wait_time = info_obj
                                                    .get("wait_time")
                                                    .and_then(|v| v.as_f64())
                                                    .unwrap_or(0.0);
                                                break;
                                            }
                                        }
                                    }
                                    wait_time as f32
                                },
                                kills: player_obj
                                    .get("kills")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as i32,
                                ground_kills: player_obj
                                    .get("groundKills")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                naval_kills: player_obj
                                    .get("navalKills")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                team_kills: player_obj
                                    .get("teamKills")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                ai_kills: player_obj
                                    .get("aiKills")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as i32,
                                ai_ground_kills: player_obj
                                    .get("aiGroundKills")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                ai_naval_kills: player_obj
                                    .get("aiNavalKills")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                assists: player_obj
                                    .get("assists")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as i32,
                                deaths: player_obj
                                    .get("deaths")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as i32,
                                capture_zone: player_obj
                                    .get("captureZone")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                damage_zone: player_obj
                                    .get("damageZone")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                score: player_obj
                                    .get("score")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0) as i32,
                                award_damage: player_obj
                                    .get("awardDamage")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                missile_evades: player_obj
                                    .get("missileEvades")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(0)
                                    as i32,
                                lineup,
                            };

                            players.push(PlayerData {
                                player_info: info,
                                replay_data,
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(ReplayResults {
        status,
        time_played,
        author_user_id,
        author,
        players,
    })
}
