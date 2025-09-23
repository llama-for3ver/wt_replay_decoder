use crate::header::ReplayHeader;
use crate::utils::hex;
use anyhow::{bail, Context, Result};
use flate2::read::ZlibDecoder;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Cursor, Read, Write};
use std::process::{Command, Stdio};

/// Reads a variable-length size prefix from the stream.
pub fn read_variable_length_size<R: Read>(stream: &mut R) -> Result<Option<(u32, usize)>> {
    let mut buf = [0u8; 1];

    // read the first byte
    match stream.read(&mut buf)? {
        0 => return Ok(None), // clean EOF
        1 => {}
        _ => bail!("Unexpected read count when reading first byte of size prefix"),
    }
    let first_byte = buf[0];
    let mut prefix_bytes_read: usize = 1;
    let payload_size: i64;

    if (first_byte & 0x80) != 0 {
        // High bit SET (1xxxxxxx)
        if (first_byte & 0x40) == 0 {
            // 10xxxxxx -> 1 byte total
            payload_size = (first_byte & 0x7F) as i64;
        } else {
            // 11xxxxxx -> Invalid case
            bail!(
                "Invalid first size prefix byte encountered: {:#02x}",
                first_byte
            );
        }
    } else {
        // High bit CLEAR (0xxxxxxx)
        if (first_byte & 0x40) != 0 {
            // 01xxxxxx -> 2 bytes total
            let mut b1_buf = [0u8; 1];
            stream
                .read_exact(&mut b1_buf)
                .context("Failed to read 2nd byte of 2-byte size prefix")?;
            prefix_bytes_read += 1;
            payload_size = (((first_byte as i64) << 8) | (b1_buf[0] as i64)) ^ 0x4000;
        } else if (first_byte & 0x20) != 0 {
            // 001xxxxx -> 3 bytes total
            let mut b1_b2_buf = [0u8; 2];
            stream
                .read_exact(&mut b1_b2_buf)
                .context("Failed to read bytes 2-3 of 3-byte size prefix")?;
            prefix_bytes_read += 2;
            payload_size = (((first_byte as i64) << 16)
                | ((b1_b2_buf[0] as i64) << 8)
                | (b1_b2_buf[1] as i64))
                ^ 0x200000;
        } else if (first_byte & 0x10) != 0 {
            // 0001xxxx -> 4 bytes total
            let mut b1_b3_buf = [0u8; 3];
            stream
                .read_exact(&mut b1_b3_buf)
                .context("Failed to read bytes 2-4 of 4-byte size prefix")?;
            prefix_bytes_read += 3;
            payload_size = (((first_byte as i64) << 24)
                | ((b1_b3_buf[0] as i64) << 16)
                | ((b1_b3_buf[1] as i64) << 8)
                | (b1_b3_buf[2] as i64))
                ^ 0x10000000;
        } else {
            // 0000xxxx -> 5 bytes total
            let mut b1_b4_buf = [0u8; 4];
            stream
                .read_exact(&mut b1_b4_buf)
                .context("Failed to read bytes 2-5 of 5-byte size prefix")?;
            prefix_bytes_read += 4;
            // little Endian u32 - use stdlib method
            payload_size = u32::from_le_bytes(b1_b4_buf) as i64;
        }
    }

    if payload_size < 0 {
        warn!(
            "Calculated negative payload size ({}). This bodes ill.",
            payload_size
        );
    }

    let final_size = payload_size.try_into().with_context(|| {
        format!(
            "Payload size {} cannot fit into u32 (prefix starts with {:#02x})",
            payload_size, first_byte
        )
    })?;

    Ok(Some((final_size, prefix_bytes_read)))
}

/// Reads packet type and timestamp from the start of a DECOMPRESSED stream/buffer.
/// Returns `Ok(Some((packet_type, timestamp_ms, bytes_read)))` or `Ok(None)` on EOF.
pub fn read_packet_header_from_stream<R: Read>(
    stream: &mut R,
    last_timestamp_ms: u32,
) -> Result<Option<(u8, u32, usize)>> {
    let mut first_byte_buf = [0u8; 1];

    match stream.read(&mut first_byte_buf)? {
        0 => return Ok(None), // clean EOF
        1 => {}
        _ => bail!("Unexpected read count reading first byte of packet header"),
    }
    let first_byte = first_byte_buf[0];
    let mut bytes_read_for_header = 1;
    let mut timestamp_ms = last_timestamp_ms;
    let packet_type_val: u8;

    if (first_byte & 0x10) != 0 {
        // timestamp didn't change
        packet_type_val = first_byte ^ 0x10;
    } else {
        // timestamp DID change
        packet_type_val = first_byte;
        let mut ts_bytes = [0u8; 4];
        match stream.read_exact(&mut ts_bytes) {
            Ok(_) => {
                timestamp_ms = u32::from_le_bytes(ts_bytes);
                bytes_read_for_header += 4;
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                warn!("Unexpected EOF reading timestamp after type byte {:#02x}. Using last known timestamp.", packet_type_val);
                return Ok(Some((packet_type_val, timestamp_ms, bytes_read_for_header)));
            }
            Err(e) => {
                return Err(e).context("Failed to read timestamp bytes");
            }
        }
    }

    Ok(Some((packet_type_val, timestamp_ms, bytes_read_for_header)))
}

/// Process replay data (potentially compressed) from a byte slice.
///
/// This function takes raw data, a start offset, and a flag indicating whether
/// to skip zlib decompression.
/// Returns information about the processing results.
pub fn process_replay_data(
    data: &[u8],
    start_offset: u64,
    skip_zlib: bool,
) -> Result<ParsedReplay> {
    // Validate start_offset before slicing
    if start_offset > data.len() as u64 {
        bail!(
            "Start offset {:#0x} is beyond the data length ({} bytes)",
            start_offset,
            data.len()
        );
    }
    let input_data = &data[start_offset as usize..];

    let mut reader = std::io::BufReader::new(create_reader(input_data, skip_zlib)?);
    if !skip_zlib {
        let peeked = reader.fill_buf().unwrap_or(&[]);
        if peeked.len() >= 3 {
            // second bytes seems to be E<anything> (E2, E6 i've seen)
            // not sure why.
            // aditionally, some replays don't have what is matched...
            if peeked[0] != 0x40 || peeked[2] != 0x08 {
                warn!("Decompressed replay stream does not start with expected bytes.");
            }
        }
    }

    let mut stats = ParsedReplay::default();
    let mut last_timestamp_ms: u32 = 0;

    loop {
        debug!(
            "Processing Packet {} (Decompressed bytes read so far: {}) ---",
            stats.packet_count, stats.total_decompressed_bytes
        );

        let (decompressed_payload_size, prefix_bytes_read) =
            match read_variable_length_size(&mut reader) {
                Ok(Some((size, bytes_read))) => (size, bytes_read),
                Ok(None) => {
                    debug!("EOF reached while reading packet size prefix. End of stream.");
                    break;
                }
                Err(e) => {
                    if let Some(io_err) = e.downcast_ref::<io::Error>() {
                        if io_err.kind() == io::ErrorKind::UnexpectedEof {
                            warn!("Incomplete packet size prefix at end of stream: {}", e);
                            break; // treat as EOF
                        }
                    }
                    error!("Error reading packet size prefix: {:?}", e);
                    bail!("Failed to read or parse packet size prefix");
                }
            };

        debug!(
            "Read size prefix ({} decomp. bytes): Expected payload size = {} bytes",
            prefix_bytes_read, decompressed_payload_size
        );
        stats.total_decompressed_bytes += prefix_bytes_read as u64;

        if decompressed_payload_size == 0 {
            warn!("Encountered zero-size packet payload. Continuing.");
        }

        let mut packet_data_with_header = vec![0u8; decompressed_payload_size as usize];
        let total_bytes_read_for_payload;

        match reader.read_exact(&mut packet_data_with_header) {
            Ok(_) => {
                total_bytes_read_for_payload = decompressed_payload_size as usize;
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                warn!(
                    "Incomplete packet payload read. Expected {}, stream ended early.",
                    decompressed_payload_size
                );
                packet_data_with_header.fill(0);
                let mut temp_reader = (&mut reader).take(decompressed_payload_size as u64);
                match temp_reader.read_to_end(&mut packet_data_with_header) {
                    Ok(bytes_actually_read) => {
                        total_bytes_read_for_payload = bytes_actually_read;
                        packet_data_with_header.truncate(total_bytes_read_for_payload);
                        warn!(
                            "Read {} bytes of partial payload.",
                            total_bytes_read_for_payload
                        );
                    }
                    Err(read_err) => {
                        error!("Error attempting to read partial payload: {:?}", read_err);
                        bail!("Failed to read partial packet payload after EOF detected");
                    }
                }
                if total_bytes_read_for_payload == 0 {
                    info!("No payload data read after size prefix indicated > 0. Stopping.");
                    break;
                }
            }
            Err(e) => {
                error!("I/O error reading packet payload: {:?}", e);
                bail!("Failed to read packet payload");
            }
        }

        stats.total_decompressed_bytes += total_bytes_read_for_payload as u64;

        if total_bytes_read_for_payload > 0 {
            let mut payload_cursor = Cursor::new(&packet_data_with_header);

            match read_packet_header_from_stream(&mut payload_cursor, last_timestamp_ms) {
                Ok(Some((packet_type_val, timestamp_ms, header_bytes_read))) => {
                    debug!(
                        "Parsed Header ({} bytes): Type={}, Timestamp={}ms",
                        header_bytes_read, packet_type_val, timestamp_ms
                    );
                    last_timestamp_ms = timestamp_ms;

                    let header_len = header_bytes_read;
                    let payload_content = &packet_data_with_header[header_len..];
                    let payload_size_actual = payload_content.len();

                    debug!(
                        "Payload Content ({} bytes): {}{}",
                        payload_size_actual,
                        hex::encode(&payload_content[..std::cmp::min(payload_size_actual, 50)]),
                        if payload_size_actual > 50 { "..." } else { "" }
                    );

                    stats.packets.push(PacketInfo {
                        packet_type: match packet_type_val {
                            // apparently this is what you're supposed to do...?
                            0 => ReplayPacketType::EndMarker,
                            1 => ReplayPacketType::StartMarker,
                            2 => ReplayPacketType::AircraftSmall,
                            3 => ReplayPacketType::Chat,
                            4 => ReplayPacketType::MPI,
                            5 => ReplayPacketType::NextSegment,
                            6 => ReplayPacketType::ECS,
                            7 => ReplayPacketType::Snapshot,
                            8 => ReplayPacketType::ReplayHeaderInfo,
                            _ => ReplayPacketType::Unknown,
                        },
                        timestamp_ms,
                        payload: payload_content.to_vec(),
                    });

                    if packet_type_val == 3 {
                        if let Some(chat_info) = parse_chat_packet(payload_content, timestamp_ms) {
                            stats.chat_messages.push(chat_info);
                        }
                    }
                }
                Ok(None) => {
                    warn!("Unexpected EOF reading packet header from payload buffer. Skipping payload.");
                    if total_bytes_read_for_payload < decompressed_payload_size as usize {
                        info!("Partial payload likely caused header read failure. Stopping.");
                        break;
                    }
                }
                Err(e) => {
                    error!("Error reading packet header from payload data: {:?}", e);
                    if total_bytes_read_for_payload < decompressed_payload_size as usize {
                        info!("Partial payload likely caused header read failure. Stopping.");
                        break;
                    }
                    bail!("Failed to parse packet header from payload");
                }
            }
        } else if decompressed_payload_size > 0 {
            info!("No payload data could be read even after partial attempt. Stopping.");
            break;
        }

        stats.packet_count += 1;
    }

    info!(
        "Processed {} packets ({} bytes)",
        stats.packet_count, stats.total_decompressed_bytes
    );

    if skip_zlib {
        stats.final_offset = start_offset + stats.total_decompressed_bytes;
        info!(
            "Final position in input (uncompressed): {:#0x}",
            stats.final_offset
        );
    } else {
        // We cannot reliably get the exact compressed position after using Box<dyn Read>
        // Report the total decompressed bytes instead.
        info!("Cannot determine exact final compressed offset after processing.");
        info!(
            "Total decompressed bytes processed: {}",
            stats.total_decompressed_bytes
        );
        // Set final_offset to 0 or some indicator value? For now, leave it as 0 (default).
        stats.final_offset = 0; // Indicate unknown compressed end offset
    }

    Ok(stats)
}

/// Creates the appropriate reader (direct or zlib) based on the flag.
fn create_reader<'a>(input_data: &'a [u8], skip_zlib: bool) -> Result<Box<dyn Read + 'a>> {
    let cursor = Cursor::new(input_data);
    let reader: Box<dyn Read + 'a> = if skip_zlib {
        info!("Processing stream directly (zlib decoding skipped).");
        Box::new(cursor)
    } else {
        info!("Processing stream with zlib decoder.");
        Box::new(ZlibDecoder::new(cursor))
    };
    Ok(reader)
}

/// Processes the replay stream provided as a byte slice.
pub fn process_replay_stream(
    replay_data: &[u8],
    start_offset: u64,
    skip_zlib: bool,
) -> Result<ParsedReplay> {
    if start_offset > 0 {
        info!(
            "Seeking to stream offset {:#0x} ({}) in input data.",
            start_offset, start_offset
        );
        if skip_zlib {
            info!("Will read raw packet data from this offset.");
        }
    } else {
        info!("Starting processing from beginning of input data (offset 0).");
    }

    let stats = process_replay_data(replay_data, start_offset, skip_zlib)?;

    Ok(stats)
}

/// Processes the replay stream with header information for complete parsing including results.
pub fn process_replay_stream_with_header(
    replay_data: &[u8],
    header: &ReplayHeader,
    start_offset: u64,
    skip_zlib: bool,
) -> Result<ParsedReplay> {
    if start_offset > 0 {
        info!(
            "Seeking to stream offset {:#0x} ({}) in input data.",
            start_offset, start_offset
        );
        if skip_zlib {
            info!("Will read raw packet data from this offset.");
        }
    } else {
        info!("Starting processing from beginning of input data (offset 0).");
    }

    let mut stats = process_replay_data(replay_data, start_offset, skip_zlib)?;

    // Attempt to parse end-of-replay results if rez_offset is available
    if header.rez_offset > 0 && header.rez_offset < replay_data.len() as u32 {
        info!(
            "Attempting to parse end-of-replay results at offset {}",
            header.rez_offset
        );
        stats.replay_results = parse_replay_results(replay_data, header.rez_offset as usize);

        if stats.replay_results.is_some() {
            info!("Successfully parsed end-of-replay results");
        } else {
            warn!("Failed to parse end-of-replay results (compression not yet implemented)");
        }
    } else {
        warn!("No valid rez_offset found in header, skipping result parsing");
    }

    Ok(stats)
}

/// The result of a parsed replay.
#[derive(Debug, Default)]
pub struct ParsedReplay {
    /// Total number of packets processed.
    pub packet_count: u64,
    /// Total bytes read *after* decompression (if any).
    /// If zlib is skipped, this is raw bytes read.
    pub total_decompressed_bytes: u64,
    pub final_offset: u64,
    /// List of packets.
    pub packets: Vec<PacketInfo>,
    /// List of chat messages.
    pub chat_messages: Vec<ChatInfo>,
    /// End-of-replay results data (if available).
    pub replay_results: Option<ReplayResults>,
}

/// Complete replay results containing battle outcome and player statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResults {
    /// Battle status (e.g., "won", "lost", "left").
    pub status: String,
    /// Time played in seconds.
    pub time_played: f64,
    /// User ID of the replay author.
    pub author_user_id: String,
    /// Username of the replay author.
    pub author: String,
    /// List of players and their performance data.
    pub players: Vec<PlayerData>,
}

/// Complete player information including profile and performance data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerData {
    /// Player profile information.
    pub player_info: PlayerInfo,
    /// Player performance statistics.
    pub replay_data: PlayerReplayData,
}

/// Player profile information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInfo {
    /// Unique user ID.
    pub user_id: String,
    /// Display username.
    pub username: String,
    /// Squadron/clan ID.
    pub squadron_id: String,
    /// Squadron/clan tag.
    pub squadron_tag: String,
    /// Gaming platform.
    pub platform: String,
}

/// Player performance statistics for the replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerReplayData {
    pub user_id: String,
    pub squad: i32,
    pub auto_squad: bool,
    pub team: i32,
    pub wait_time: f32,
    pub kills: i32,
    pub ground_kills: i32,
    pub naval_kills: i32,
    pub team_kills: i32,
    pub ai_kills: i32,
    pub ai_ground_kills: i32,
    pub ai_naval_kills: i32,
    pub assists: i32,
    pub deaths: i32,
    pub capture_zone: i32,
    pub damage_zone: i32,
    pub score: i32,
    pub award_damage: i32,
    pub missile_evades: i32,
    pub lineup: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPacketType {
    /// End of replay marker.
    EndMarker = 0,
    /// Start of replay marker.
    StartMarker = 1, // ?
    /// Aircraft state updates (positions, velocity, controls, etc.)
    AircraftSmall = 2,
    /// Chat messages - Sender, message, flags
    Chat = 3,
    /// Wrapped MPI messages (ObjectID, MessageID, payload)
    MPI = 4,
    /// Next segment marker.
    NextSegment = 5,
    /// ECS network data.
    ECS = 6,
    /// Full game state snapshot, can't find it used however.
    Snapshot = 7,
    /// Initial header/settings data duplication.
    ReplayHeaderInfo = 8,
    /// Unknown packet type.
    Unknown = 255, // Using 255 for unknown since -1 doesn't fit u8
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct PacketInfo {
    pub packet_type: ReplayPacketType,
    /// seems to be lying
    pub timestamp_ms: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ChatInfo {
    /// Timestamp in milliseconds
    pub timestamp_ms: u32,
    /// The nick of the sender.
    pub sender: String,
    /// The message content
    pub message: String,
    /// The type of channel. Believe it's for all, team, squad etc.
    pub channel_type: Option<u8>,
    /// Whether the sender is an enemy
    pub is_enemy: Option<u8>,
}

/// Parses the payload of a chat packet.
pub fn parse_chat_packet(payload: &[u8], timestamp_ms: u32) -> Option<ChatInfo> {
    let mut cursor = Cursor::new(payload);

    fn read_u8(cur: &mut Cursor<&[u8]>) -> Result<u8> {
        let mut buf = [0u8; 1];
        cur.read_exact(&mut buf).context("Failed to read byte")?;
        Ok(buf[0])
    }

    fn read_string(cur: &mut Cursor<&[u8]>, len: usize, full_len: usize) -> Result<String> {
        let current_pos = cur.position() as usize;
        if current_pos + len > full_len {
            bail!("Payload too short for string of length {}", len);
        }
        let mut buf = vec![0u8; len];
        cur.read_exact(&mut buf)?;
        String::from_utf8(buf).context("Failed to decode UTF-8 string")
    }

    if payload.is_empty() {
        warn!("[Chat Type 3] Empty payload.");
        return None;
    }

    let mut skip_buf = [0u8; 1];
    if let Err(e) = cursor.read_exact(&mut skip_buf) {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            warn!("[Chat Type 3] Payload was empty when trying to read subtype/flag byte.");
        } else {
            error!("[Chat Type 3] Error reading subtype/flag byte: {:?}", e);
        }
        return None;
    }

    if cursor.position() as usize >= payload.len() {
        warn!("[Chat Type 3] Payload contained only the initial subtype/flag byte.");
        return None;
    }

    match (|| -> Result<ChatInfo> {
        let sender_len = read_u8(&mut cursor)? as usize;
        let sender_name = read_string(&mut cursor, sender_len, payload.len())?;

        let message_len = read_u8(&mut cursor)? as usize;
        let message = read_string(&mut cursor, message_len, payload.len())?;

        let remaining = payload.len() as u64 - cursor.position();
        let channel_type = if remaining >= 1 {
            Some(read_u8(&mut cursor)?)
        } else {
            None
        };
        let is_enemy = if remaining >= 2 {
            Some(read_u8(&mut cursor)?)
        } else {
            None
        };

        debug!(
            "[Chat] Decoded message - Timestamp: {} ms, Sender: '{}', Message: '{}', Channel: {:?}, Enemy: {:?}",
            timestamp_ms, sender_name, message, channel_type, is_enemy
        );

        Ok(ChatInfo {
            timestamp_ms,
            sender: sender_name,
            message,
            channel_type,
            is_enemy,
        })
    })() {
        Ok(chat_info) => Some(chat_info),
        Err(e) => {
            error!(
                "[Chat Type 3] Error parsing packet payload: {:?}. Payload start: {}...",
                e,
                hex::encode(&payload[..std::cmp::min(payload.len(), 30)])
            );
            None
        }
    }
}

/// Parses end-of-replay results from offset.
pub fn parse_replay_results(data: &[u8], rez_offset: usize) -> Option<ReplayResults> {
    if rez_offset >= data.len() {
        error!(
            "Rez offset {} is beyond data length {}",
            rez_offset,
            data.len()
        );
        return None;
    }

    let compressed_data = &data[rez_offset..];

    debug!(
        "Found compressed data at offset {} with {} bytes",
        rez_offset,
        compressed_data.len()
    );

    // Use wt_ext_cli to decompress the data
    let json_data = match decompress_blk(compressed_data) {
        Ok(data) => data,
        Err(e) => {
            error!("Failed to decompress replay results data: {}", e);
            return None;
        }
    };

    match parse_replay_results_json(&json_data) {
        Ok(results) => Some(results),
        Err(e) => {
            error!("Failed to parse replay results JSON: {}", e);
            None
        }
    }
}

fn decompress_blk(compressed_data: &[u8]) -> Result<String> {
    // FIXME: Use Rusty API (wt_blk)
    let mut child = Command::new("wt_ext_cli")
        .args(&[
            "--unpack_raw_blk",
            "--stdout",
            "--stdin",
            "--format",
            "Json",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to start wt_ext_cli. Make sure it's installed and in PATH.")?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(compressed_data)
            .context("Failed to write data to wt_ext_cli stdin")?;
    }

    let output = child
        .wait_with_output()
        .context("Failed to wait for wt_ext_cli to complete")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "wt_ext_cli failed with status {}: {}",
            output.status,
            stderr
        );
    }

    let json_output =
        String::from_utf8(output.stdout).context("Couldn't parse JSON output (UTF-8)")?;

    // info!("{}", json_output);

    Ok(json_output)
}

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

                        // Find matching player info
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
                                    // wait_time comes from player info, not player data
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
