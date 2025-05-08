use anyhow::{Context, Result};
use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Difficulty {
    pub unknown_nibble: u8,
    pub difficulty_value: u8,
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
    /// something about nukes.
    /// believe it's always `killStreaksAircraftOrHelicopter_1`
    /// at the appropriate BR & mode.
    pub battle_kill_streak: String,
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
        writeln!(f, "Location Name: {}", self.loc_name)?;
        writeln!(f, "Start Time: {}", self.start_time)?;
        writeln!(f, "Time Limit: {}", self.time_limit)?;
        writeln!(f, "Score Limit: {}", self.score_limit)?;
        writeln!(f, "Battle Class: {}", self.battle_class)?;
        writeln!(f, "Battle Kill Streak: {}", self.battle_kill_streak)?;
        Ok(())
    }
}


/// Parses the header of a replay file.
/// Takes a *path* ATM.
pub fn parse_header(file_path: &Path) -> Result<ReplayHeader> {
    let mut file = File::open(file_path)
        .with_context(|| format!("Failed to open replay file: {:?}", file_path))?;

    let mut buffer = [0u8; 4];

    // Read magic
    file.read_exact(&mut buffer)?;
    let magic = u32::from_le_bytes(buffer);

    // Read version
    file.read_exact(&mut buffer)?;
    let version = u32::from_le_bytes(buffer);

    // Read level (128 bytes)
    let level = read_string(&mut file, 128)?;

    // Read level settings (260 bytes)
    let level_settings = read_string(&mut file, 260)?;

    // Read battle type (128 bytes)
    let battle_type = read_string(&mut file, 128)?;

    // Read environment (128 bytes)
    let environment = read_string(&mut file, 128)?;

    // Read visibility (32 bytes)
    let visibility = read_string(&mut file, 32)?;

    // Read rez offset
    file.read_exact(&mut buffer)?;
    let rez_offset = u32::from_le_bytes(buffer);

    // Read difficulty (one byte)
    let mut diff_byte = [0u8; 1];
    file.read_exact(&mut diff_byte)?;
    let difficulty = Difficulty::from_byte(diff_byte[0]);

    // Skip padding (35 bytes)
    file.seek(SeekFrom::Current(35))?;

    // Read session type
    file.read_exact(&mut buffer)?;
    let session_type = u32::from_le_bytes(buffer);

    // Skip padding (4 bytes)
    file.seek(SeekFrom::Current(4))?;

    // Read session id (8 bytes)
    let mut session_buffer = [0u8; 8];
    file.read_exact(&mut session_buffer)?;
    let session_id_hex = u64::from_le_bytes(session_buffer);

    // Skip padding (4 bytes)
    file.seek(SeekFrom::Current(4))?;

    // Read m_set_size
    file.read_exact(&mut buffer)?;
    let m_set_size = u32::from_le_bytes(buffer);

    // Skip padding (32 bytes)
    file.seek(SeekFrom::Current(32))?;

    // Read loc_name (128 bytes)
    let loc_name = read_string(&mut file, 128)?;

    // Read start_time
    file.read_exact(&mut buffer)?;
    let start_time = u32::from_le_bytes(buffer);

    // Read time_limit
    file.read_exact(&mut buffer)?;
    let time_limit = u32::from_le_bytes(buffer);

    // Read score_limit
    file.read_exact(&mut buffer)?;
    let score_limit = u32::from_le_bytes(buffer);

    // Skip padding (48 bytes)
    file.seek(SeekFrom::Current(48))?;

    // Read battle_class (128 bytes)
    let battle_class = read_string(&mut file, 128)?;

    // Read battle_kill_streak (128 bytes)
    let battle_kill_streak = read_string(&mut file, 128)?;

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
        loc_name,
        start_time,
        time_limit,
        score_limit,
        battle_class,
        battle_kill_streak,
    })
}

fn read_string(file: &mut File, max_len: usize) -> Result<String> {
    let mut buffer = vec![0u8; max_len];
    file.read_exact(&mut buffer)?;

    // find the null terminator...
    let null_pos = buffer.iter().position(|&b| b == 0).unwrap_or(max_len);

    // take bytes up to the null terminator
    let bytes = &buffer[..null_pos];

    // convert to string
    let string = String::from_utf8_lossy(bytes).into_owned();

    Ok(string)
}
