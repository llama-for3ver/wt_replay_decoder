use crate::header::{parse_header, ReplayHeader, ReplaySettings};
use crate::packet::{
    parse_award_packet, parse_chat_packet, read_packet_header, read_vlq_size, PacketInfo,
};
use crate::packet::{AwardInfo, ChatInfo, ReplayPacketType};
use crate::results::parse_replay_results_json;
use anyhow::{bail, Context, Result};
use flate2::read::ZlibDecoder;
use log::{debug, error, info, warn};
use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Cursor, Read};
use std::sync::Arc;
use wt_blk::blk;
use wt_blk::blk::file::FileType;
use wt_blk::blk::name_map::NameMap;

/// Process replay data (potentially compressed) from a byte slice.
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
            // additionally, some replays don't have what is matched...
            if peeked[0] != 0x40 || peeked[2] != 0x08 {
                warn!("Decompressed replay stream does not start with expected bytes.");
            }
        }
    }

    let mut stats = ParsedReplay::default();
    let last_timestamp_ms = 0;

    loop {
        debug!(
            "Processing Packet {} (Decompressed bytes read so far: {}) ---",
            stats.packet_count, stats.total_decompressed_bytes
        );

        let (decompressed_payload_size, prefix_bytes_read) = match read_vlq_size(&mut reader) {
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

            match read_packet_header(&mut payload_cursor, last_timestamp_ms) {
                Ok(Some((packet_type_val, timestamp_ms, header_bytes_read))) => {
                    debug!(
                        "Parsed Header ({} bytes): Type={}, Timestamp={}ms",
                        header_bytes_read, packet_type_val, timestamp_ms
                    );

                    let header_len = header_bytes_read;
                    let payload_content = &packet_data_with_header[header_len..];

                    stats.packets.push(PacketInfo {
                        packet_type: match packet_type_val {
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
                    if packet_type_val == 4 {
                        // MPI, but for now just do awards lol
                        if let Some(award_info) = parse_award_packet(payload_content, timestamp_ms)
                        {
                            stats.award_messages.push(award_info);
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
        info!("Cannot determine exact final compressed offset after processing.");
        info!(
            "Total decompressed bytes processed: {}",
            stats.total_decompressed_bytes
        );
        stats.final_offset = 0;
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
    header: Option<&ReplayHeader>,
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

    if let Some(header) = header {
        if header.settings_size > 0 {
            // (client_2) 0x000004CA = 0x01
            // https://github.com/Warthunder-Open-Source-Foundation/wt_blk/blob/master/src/blk/file.rs#L10
            // using mimi here won't work as it uses memory size, not disk.
            // also +2 is to skip over some mystery bytes
            let hdr_size = usize::try_from(header._total_length)? + 2;
            info!("Header size: {}", hdr_size);
            let settings_size = header.settings_size as usize;
            let settings_start = hdr_size;
            let settings_end = settings_start.saturating_add(settings_size);
            if settings_end <= replay_data.len() {
                info!(
                    "Parsing settings BLK at offset {} ({} bytes)",
                    settings_start, settings_size
                );
                let blk_data = &replay_data[settings_start..settings_end];
                match decompress_blk(blk_data) {
                    Ok(json) => {
                        println!("{}", &json);
                        match serde_json::from_str::<ReplaySettings>(&json) {
                            Ok(deserialized) => {
                                stats.replay_settings = Some(deserialized);
                                info!("Successfully parsed settings");
                            }
                            Err(e) => {
                                bail!("Failed to deserialize settings: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Failed to parse settings: {}", e);
                    }
                }
            } else {
                warn!(
                    "Settings range {}..{} out of data bounds",
                    settings_start, settings_end
                );
            }
        }
    }

    // Then parse end-of-replay results if available
    if let Some(header) = header {
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
    }

    Ok(stats)
}

pub fn process_parted_replay(buffers: &[Vec<u8>], skip_zlib: bool) -> Result<ParsedReplay> {
    if buffers.is_empty() {
        bail!("No replay parts provided");
    }
    let mut combined = ParsedReplay::default();
    for buf in buffers {
        let hdr = parse_header(buf).context("parsing replay segment header")?;
        let offset = hdr._total_length + 2 + hdr.settings_size as u64;
        let part = process_replay_stream(buf, offset, skip_zlib, None)?;
        combined.packet_count += part.packet_count;
        combined.total_decompressed_bytes += part.total_decompressed_bytes;
        combined.packets.extend(part.packets);
        combined.chat_messages.extend(part.chat_messages);
        combined.award_messages.extend(part.award_messages);
    }
    Ok(combined)
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
    /// List of award messages.
    pub award_messages: Vec<AwardInfo>,
    /// Parsed settings data(if available).
    pub replay_settings: Option<ReplaySettings>,
    /// End-of-replay results data (if available).
    pub replay_results: Option<ReplayResults>,
}

/// Complete replay results containing battle outcome and player statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayResults {
    /// Battle result ("won", "lost", "left").
    pub status: String,
    /// Time played in seconds.
    pub time_played: f64,
    /// User ID of the replay author.
    pub author_user_id: String,
    /// Username of the replay author.
    pub author: String,
    /// List of players and their results.
    pub players: Vec<PlayerData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerData {
    pub player_info: PlayerInfo,
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
    /// Player's platform ("win64", "macosx", etc.)
    pub platform: String,
}

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
    if compressed_data.is_empty() {
        bail!("No data provided for decompress_blk");
    }

    let zstd_dict = None;
    let nm: Option<Arc<NameMap>> = None;

    match FileType::from_byte(compressed_data[0])? {
        FileType::BBF => {}
        FileType::FAT => {}
        FileType::FAT_ZSTD => {}
        FileType::SLIM => {}
        FileType::SLIM_ZSTD => {}
        FileType::SLIM_ZST_DICT => {
            bail!("ZSTD dictionary compressed BLK not supported");
        }
    }

    let mut compressed_vec = compressed_data.to_vec();
    let mut parsed = blk::unpack_blk(&mut compressed_vec, zstd_dict, nm)
        .map_err(|e| anyhow::anyhow!("blk::unpack_blk failed: {}", e))?;

    let _ = parsed.merge_fields();
    let json_bytes = parsed
        .as_serde_json()
        .map_err(|e| anyhow::anyhow!("blk::as_serde_json failed: {}", e))?;

    let json_output = String::from_utf8(json_bytes)
        .context("Couldn't parse BLK JSON output (UTF-8 conversion failed)")?;

    Ok(json_output)
}
