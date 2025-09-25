pub mod header;
pub mod parser;
pub mod utils;

#[cfg(test)]
mod tests {
    use std::fs::read;

    use super::*;

    // we already know the offset of zlib is 0x828 for client_1
    #[test]
    /// Parse entirety of /tests/replays/client_1.wrpl.
    /// And assert chat messages are correct.
    fn test_parse_client_1() {
        let file = std::fs::read("tests/replays/client_1.wrpl").unwrap();

        let replay = parser::process_replay_stream(&file, 2088, false, None).unwrap();
        // 19 messages
        assert_eq!(replay.chat_messages.len(), 19);
        // first one is "TEST" from "kiTmalZ"
        assert_eq!(replay.chat_messages[0].message, "TEST");
        assert_eq!(replay.chat_messages[0].sender, "kiTmalZ");

        // last is "Attack the D point!" from "AceLavrinenko"
        assert_eq!(replay.chat_messages[18].message, "Attack the D point!");
        assert_eq!(replay.chat_messages[18].sender, "AceLavrinenko");

        // 17th message is channel 1 (all?)
        // " 17: Gyaru-Destroyer says
        // 'yo enemy team could you hold up on the attack we aren't loaded'
        // (channel: Some(1), enemy: Some(0))"
        assert_eq!(
            replay.chat_messages[16].message,
            "yo enemy team could you hold up on the attack we aren't loaded"
        );
        assert_eq!(replay.chat_messages[16].channel_type, Some(1));
    }

    #[test]
    /// Parse the header of /tests/replays/client_1.wrpl.
    /// And assert the header values are correct.
    ///
    fn test_parse_client_1_header() {
        let file = std::fs::read("tests/replays/client_1.wrpl").unwrap();
        let header = header::parse_header(&file).unwrap();

        assert_eq!(header.version, 101286);
        assert_eq!(header.level, "levels/avg_egypt_sinai.bin");
        assert_eq!(
            header.level_settings,
            "gamedata/missions/cta/tanks/sinai_sands/sinai_02_conq1.blk"
        );
        assert_eq!(header.battle_type, "sinai_02_Conq1");
        assert_eq!(header.environment, "noon");
        assert_eq!(header.visibility, "thin_clouds");
        assert_eq!(header.rez_offset, 3662909);
        // skip difficulty for now
        assert_eq!(header.session_type, 0);
        assert_eq!(header.session_id_hex, 335055458235795646);
        assert_eq!(header.m_set_size, 8062);
        assert_eq!(header.loc_name, "missions/_Conq1;sinai_02/name");
        assert_eq!(header.start_time, 1746008224);
        assert_eq!(header.time_limit, 25);
        assert_eq!(header.score_limit, 16000);
        assert_eq!(header.battle_class, "air_ground_Conq");
        // nuke stuff is empty as it's too low BR
        assert_eq!(header.battle_kill_streak, "");
    }

    #[test]
    /// Parse the header of /tests/replays/server_3.wrpl.
    fn test_parse_server_header() {
        let file = read("tests/replays/server_3.wrpl").unwrap();

        let header = header::parse_header(&file).unwrap();

        assert_eq!(header.version, 101286);
        assert_eq!(header.level, "levels/air_mysterious_valley.bin");
        assert_eq!(
            header.level_settings,
            "gamedata/missions/cta/planes/historical/bfd/air_mysterious_valley_wide_spawns_bfd_norespawn.blk");
        assert_eq!(
            header.battle_type,
            "air_mysterious_valley_wide_spawns_BfD_norespawn"
        );
        assert_eq!(header.environment, "noon");
        assert_eq!(header.visibility, "cloudy");
        assert_eq!(header.rez_offset, 0);
        assert_eq!(header.session_type, 0);
        assert_eq!(header.session_id_hex, 336062142732521316);
        assert_eq!(header.m_set_size, 30709);
        assert_eq!(
            header.loc_name,
            "missions/air_mysterious_valley_wide_spawns_BfD_norespawn"
        );
        assert_eq!(header.start_time, 1746242707);
        assert_eq!(header.time_limit, 25);
        assert_eq!(header.score_limit, 10400);
        assert_eq!(header.battle_class, "base_dom");
        assert_eq!(header.battle_kill_streak, "");
    }

    #[test]
    fn test_parse_client_results() {
        // This test parses the client_1.wrpl and asserts key values from replay results at the rez_offset.
        let file = std::fs::read("tests/replays/client_2.wrpl").unwrap();
        let header = header::parse_header(&file).unwrap();

        // This should match the BLK JSON, i.e. not fail
        let results = parser::parse_replay_results(&file, header.rez_offset as usize)
            .expect("parse_replay_results returned None");

        assert_eq!(results.status, "fail");
        assert_eq!(results.author, "[WTPU3] kiTmalZ");
        assert_eq!(results.time_played, 578.3303_f64);
        assert_eq!(results.author_user_id, 176625161.to_string());

        // TODO: Add more assertions for player info
        assert_eq!(results.players.len(), 18);
        assert_eq!(results.players[0].player_info.platform, "win64")
    }
}
