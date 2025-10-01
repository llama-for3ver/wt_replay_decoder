use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Cursor, Read, Seek, SeekFrom};

#[derive(Debug, Clone)]
pub struct Difficulty {
    pub unknown_nibble: u8,
    pub difficulty_value: u8,
}

pub mod difficulty_value {
    pub const ARCADE: i32 = 0x0000;
    pub const REALISTIC: i32 = 0x0101;
    pub const HARDCORE: i32 = 0x1010;
}

pub mod session_type {
    /// planes, simulator battles
    pub const AIR_SIM: u8 = 0x3c;
    /// naval battles
    pub const MARINE_BATTLE: u8 = 0x1a;
    /// random battle
    pub const RANDOM_BATTLE: u8 = 0x20;
    /// test range
    pub const CUSTOM_BATTLE: u8 = 0x40;
    /// user missions
    pub const USER_MISSION: u8 = 0x01;
}

pub mod local_player_country {
    pub const COUNTRY_USA: u8 = 0x01;
    pub const COUNTRY_GERMANY: u8 = 0x02;
    pub const COUNTRY_USSR: u8 = 0x03;
    pub const COUNTRY_BRITAIN: u8 = 0x04;
    pub const COUNTRY_JAPAN: u8 = 0x05;
    pub const COUNTRY_CHINA: u8 = 0x06;
    pub const COUNTRY_ITALY: u8 = 0x07;
    pub const COUNTRY_FRANCE: u8 = 0x08;
    pub const COUNTRY_SWEDEN: u8 = 0x09;
    pub const COUNTRY_ISRAEL: u8 = 0x0A;
}

// DifficultyCon = ct.ExprAdapter(ct.Bitwise(ct.FocusedSeq(
//     'difficulty',
//     'unk_nib' / ct.BitsInteger(4),
//     'difficulty' / ct.BitsInteger(4))),
//     lambda obj, context: Difficulty(obj),
//     no_encoder
// )
impl Difficulty {
    fn from_byte(byte: u8) -> Self {
        Difficulty {
            unknown_nibble: (byte >> 4) & 0x0F, // high 4 bits
            difficulty_value: byte & 0x0F,      // low 4 bits
        }
    }
}

/// The header of a replay file.
/// Should be agnostic towards server or client.
#[derive(Debug, Clone)]
pub struct ReplayHeader {
    /// The magic bytes used for .wrpl.
    pub magic: u32,
    /// The version of the replay file.
    /// May or may not be seperate from WT version.
    pub version: u32,
    /// the bin file of the level.
    pub level: String,
    /// the blk file for game config.
    pub level_settings: String,
    /// what type of battle (i.e. battle, conquest, domination).
    pub battle_type: String,
    /// time of day (and other factors?).
    pub environment: String,
    /// cloud conditions, such as fog or light clouds.
    pub visibility: String,
    /// something about offsets
    /// obviously
    pub rez_offset: u32,
    /// ???
    pub difficulty: Difficulty,
    /// ???
    // might actually be 0-2+, arcade, realistic, sim (& more?)
    // as i've seen this before.
    // https://github.com/llama-for3ver/wtjs/blob/main/src/proto/profile/WTProfile.proto
    pub session_type: u32,
    /// the session id of the replay.
    /// seen in both decimal and hex.
    pub session_id_hex: u64,
    /// ???
    pub m_set_size: u32,
    /// settings blk size
    pub settings_size: u32,
    /// ???
    pub loc_name: String,
    /// since epoch.
    pub start_time: u32,
    /// game time limit in minutes.
    pub time_limit: u32,
    /// ???
    pub score_limit: u32,
    /// vehicles usable?
    pub battle_class: String,
    /// `killStreaksAircraftOrHelicopter_1` if nukes are available
    pub battle_kill_streak: String,
    /// this is a calculated value, not an actual field.
    pub _total_length: u64,
}

// FIXME: Hacky workarounds for missing fields.

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize)]
/// Replay settings, note that this doesn't follow naming convention. It is also prone
/// to panicing.
pub struct ReplaySettings {
    pub level: String,
    #[serde(rename = "type")]
    pub mode_type: String,
    pub environment: String,
    pub weather: String,
    pub locName: String,
    pub locDesc: String,
    pub scoreLimit: u32,
    pub timeLimit: u32,
    pub deathPenaltyMul: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postfix: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctaCaptureZoneEqualPenaltyMul: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowedKillStreaks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub randomSpawnTeams: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remapAiTankModels: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battleAreaColorPreset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub showTacticalMapCellSize: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_axis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country_allies: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restoreType: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub optionalTakeOff: Option<bool>,
    pub allowedUnitTypes: AllowedUnitTypes,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mission: Option<Vec<MissionSettings>>,
    pub stars: StarsSettings,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowedUnitTypes {
    pub isAirplanesAllowed: bool,
    pub isTanksAllowed: bool,
    pub isShipsAllowed: bool,
    pub isHelicoptersAllowed: bool,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionSettings {
    pub difficulty: String,
    pub useAlternativeMapCoord: bool,
    pub scoreLimit: u32,
    pub randomSpawnTeams: bool,
    pub remapAiTankModels: bool,
}

#[allow(non_snake_case)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StarsSettings {
    pub latitude: f32,
    pub longitude: f32,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub localTime: f32,
}

impl fmt::Display for ReplayHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Magic bytes: {:#x}", self.magic)?;
        writeln!(f, "Version: {}", self.version)?;
        writeln!(f, "Level: {}", self.level)?;
        writeln!(f, "Level Settings: {}", self.level_settings)?;
        writeln!(f, "Battle Type: {}", self.battle_type)?;
        writeln!(f, "Environment: {}", self.environment)?;
        writeln!(f, "Visibility: {}", self.visibility)?;
        writeln!(f, "Rez Offset: {}", self.rez_offset)?;
        writeln!(
            f,
            "Difficulty: {} (unknown: {})",
            self.difficulty.difficulty_value, self.difficulty.unknown_nibble
        )?;
        writeln!(f, "Session Type: {}", self.session_type)?;
        writeln!(
            f,
            "Session ID: {:#x} ({})",
            self.session_id_hex, self.session_id_hex
        )?;
        writeln!(f, "MSet Size: {}", self.m_set_size)?;
        writeln!(f, "Settings Size: {}", self.settings_size)?;
        writeln!(f, "Location Name: {}", self.loc_name)?;
        writeln!(f, "Start Time: {}", self.start_time)?;
        writeln!(f, "Time Limit: {}", self.time_limit)?;
        writeln!(f, "Score Limit: {}", self.score_limit)?;
        writeln!(f, "Battle Class: {}", self.battle_class)?;
        writeln!(f, "Battle Kill Streak: {}", self.battle_kill_streak)?;
        Ok(())
    }
}

/// Parses the header of a replay file from a byte slice.
pub fn parse_header(data: &[u8]) -> Result<ReplayHeader> {
    let mut cursor = Cursor::new(data);
    let mut buffer = [0u8; 4];

    // Read magic
    cursor.read_exact(&mut buffer)?;
    let magic = u32::from_le_bytes(buffer);

    // Read version
    cursor.read_exact(&mut buffer)?;
    let version = u32::from_le_bytes(buffer);

    // Read level (128 bytes)
    let level = read_string(&mut cursor, 128)?;

    // Read level settings (260 bytes)
    let level_settings = read_string(&mut cursor, 260)?;

    // Read battle type (128 bytes)
    let battle_type = read_string(&mut cursor, 128)?;

    // Read environment (128 bytes)
    let environment = read_string(&mut cursor, 128)?;

    // Read visibility (32 bytes)
    let visibility = read_string(&mut cursor, 32)?;

    // Read rez offset
    cursor.read_exact(&mut buffer)?;
    let rez_offset = u32::from_le_bytes(buffer);

    // Read difficulty (one byte)
    let mut diff_byte = [0u8; 1];
    cursor.read_exact(&mut diff_byte)?;
    let difficulty = Difficulty::from_byte(diff_byte[0]);

    // Skip padding (35 bytes)
    cursor.seek(SeekFrom::Current(35))?;

    // Read session type
    cursor.read_exact(&mut buffer)?;
    let session_type = u32::from_le_bytes(buffer);

    // Skip padding (4 bytes)
    cursor.seek(SeekFrom::Current(4))?;

    // Read session id (8 bytes)
    let mut session_buffer = [0u8; 8];
    cursor.read_exact(&mut session_buffer)?;
    let session_id_hex = u64::from_le_bytes(session_buffer);

    // Skip padding (4 bytes)
    cursor.seek(SeekFrom::Current(4))?;

    // Read m_set_size
    cursor.read_exact(&mut buffer)?;
    let m_set_size = u32::from_le_bytes(buffer);

    // Read settings size
    // FIXME: might be 16?
    cursor.read_exact(&mut buffer)?;
    let settings_size = u32::from_le_bytes(buffer);

    // Skip padding (32->28 bytes)
    cursor.seek(SeekFrom::Current(28))?;

    // Read loc_name (128 bytes)
    let loc_name = read_string(&mut cursor, 128)?;

    // Read start_time
    cursor.read_exact(&mut buffer)?;
    let start_time = u32::from_le_bytes(buffer);

    // Read time_limit
    cursor.read_exact(&mut buffer)?;
    let time_limit = u32::from_le_bytes(buffer);

    // Read score_limit
    cursor.read_exact(&mut buffer)?;
    let score_limit = u32::from_le_bytes(buffer);

    // Skip padding (48 bytes)
    cursor.seek(SeekFrom::Current(48))?;

    // Read battle_class (128 bytes)
    let battle_class = read_string(&mut cursor, 128)?;

    // Read battle_kill_streak (128 bytes)
    let battle_kill_streak = read_string(&mut cursor, 128)?;

    let _total_length = cursor
        .seek(SeekFrom::Current(0))?
        .try_into()
        .and_then(|_: u64| Ok(cursor.position()))?;

    Ok(ReplayHeader {
        magic,
        version,
        level,
        level_settings,
        battle_type,
        environment,
        visibility,
        rez_offset,
        difficulty,
        session_type,
        session_id_hex,
        m_set_size,
        settings_size,
        loc_name,
        start_time,
        time_limit,
        score_limit,
        battle_class,
        battle_kill_streak,
        _total_length,
    })
}

fn read_string<R: Read + Seek>(reader: &mut R, max_len: usize) -> Result<String> {
    let mut buffer = vec![0u8; max_len];
    reader.read_exact(&mut buffer)?;

    // find the null terminator...
    let null_pos = buffer.iter().position(|&b| b == 0).unwrap_or(max_len);

    // take bytes up to the null terminator
    let bytes = &buffer[..null_pos];

    // convert to string
    let string = String::from_utf8_lossy(bytes).into_owned();

    Ok(string)
}
