use anyhow::Context;
use anyhow::{bail, Result};
use log::{debug, error, warn};
use std::io::{self, Cursor, Read};

use crate::utils::hex;

/// Reads a variable-length size prefix from the stream.
pub fn read_vlq_size<R: Read>(stream: &mut R) -> Result<Option<(u32, usize)>> {
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

pub fn read_packet_header<R: Read>(
    stream: &mut R,
    last_timestamp_ticks: u32,
) -> Result<Option<(u8, u32, usize)>> {
    let mut first_byte_buf = [0u8; 1];

    match stream.read(&mut first_byte_buf)? {
        0 => return Ok(None), // clean EOF
        1 => {}
        _ => bail!("Unexpected read count reading first byte of packet header"),
    }
    let first_byte = first_byte_buf[0];
    let mut bytes_read_for_header = 1;
    let mut timestamp_ticks = last_timestamp_ticks;
    let packet_type_val: u8;

    if (first_byte & 0x10) != 0 {
        // timestamp didn't change
        packet_type_val = first_byte ^ 0x10;
    } else {
        packet_type_val = first_byte;
        let mut ts_bytes = [0u8; 4];
        match stream.read_exact(&mut ts_bytes) {
            Ok(_) => {
                timestamp_ticks = u32::from_le_bytes(ts_bytes);
                bytes_read_for_header += 4;
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                warn!("Unexpected EOF reading timestamp after type byte {:#02x}. Using last known timestamp.", packet_type_val);
                return Ok(Some((
                    packet_type_val,
                    timestamp_ticks,
                    bytes_read_for_header,
                )));
            }
            Err(e) => {
                return Err(e).context("Failed to read timestamp bytes");
            }
        }
    }

    Ok(Some((
        packet_type_val,
        timestamp_ticks,
        bytes_read_for_header,
    )))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayPacketType {
    /// End of replay marker.
    EndMarker = 0,
    /// Start of replay marker.
    StartMarker = 1,
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
    /// Full game state snapshot.
    Snapshot = 7,
    /// Initial header/settings data duplication.
    ReplayHeaderInfo = 8,
    /// Unknown packet type.
    Unknown = 255,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct PacketInfo {
    pub packet_type: ReplayPacketType,
    pub timestamp_ticks: u32,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct ChatInfo {
    /// Timestamp in milliseconds
    pub timestamp_ticks: u32,
    /// The nick of the sender.
    pub sender: String,
    /// The message content
    pub message: String,
    /// The type of channel. Believe it's for all, team, squad etc.
    pub channel_type: Option<u8>,
    /// Whether the sender is an enemy
    pub is_enemy: Option<u8>,
}

/// Information about an award packet (MPI)
#[derive(Debug)]
pub struct AwardInfo {
    /// Timestamp in milliseconds.
    pub timestamp_ticks: u32,
    /// Award type byte.
    pub award_type: u8,
    /// Player index byte.
    pub player: u8,
    /// Award name.
    pub award_name: String,
}

/// Parses the payload of a chat packet. This is type `4`.
pub fn parse_chat_packet(payload: &[u8], timestamp_ticks: u32) -> Option<ChatInfo> {
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
            timestamp_ticks, sender_name, message, channel_type, is_enemy
        );

        Ok(ChatInfo {
            timestamp_ticks,
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

/// Parses the payload of an award MPI packet (0x00025878).
pub fn parse_award_packet(payload: &[u8], timestamp_ticks: u32) -> Option<AwardInfo> {
    let mut cursor = Cursor::new(payload);
    // signature: 4 bytes
    let mut sig = [0u8; 4];
    if cursor.read_exact(&mut sig).is_err() {
        return None;
    }
    if sig != [0x00, 0x02, 0x58, 0x78] {
        return None;
    }
    let mut marker = [0u8; 1];
    cursor.read_exact(&mut marker).ok()?;

    // AwardType (1 byte)
    let mut at = [0u8; 1];
    cursor.read_exact(&mut at).ok()?;
    let award_type = at[0];

    // Skip marker 0x003e (2 bytes)
    let mut b003e = [0u8; 2];
    if cursor.read_exact(&mut b003e).is_err() {
        return None;
    }

    // player
    let mut b_player = [0u8; 1];
    if cursor.read_exact(&mut b_player).is_err() {
        return None;
    }
    let player = b_player[0];
    // Skip marker 0x000000 (3 bytes)
    let mut b000000 = [0u8; 3];
    if cursor.read_exact(&mut b000000).is_err() {
        return None;
    }

    // award name
    let mut len_buf = [0u8; 1];
    if cursor.read_exact(&mut len_buf).is_err() {
        return None;
    }

    let name_len = len_buf[0] as usize;
    let mut name_buf = vec![0u8; name_len];
    if cursor.read_exact(&mut name_buf).is_err() {
        return None;
    }
    let award_name = String::from_utf8_lossy(&name_buf).into_owned();

    // remainder
    let _ = cursor.read_to_end(&mut Vec::new());
    Some(AwardInfo {
        timestamp_ticks,
        award_type,
        player,
        award_name,
    })
}

#[derive(Debug)]
pub struct KillInfo {
    pub timestamp_ticks: u32,
    pub control: u8,
    pub damage_type: u8,
    pub killer_id: u8,
    pub killer_vehicle: String,
}

/// Parses the payload of a kill MPI packet.
pub fn parse_kill_packet(payload: &[u8], timestamp_ticks: u32) -> Option<KillInfo> {
    let mut cursor = Cursor::new(payload);

    let mut signature = [0u8; 4];
    let _ = cursor.read_exact(&mut signature);

    // Always 0xf0 marker (skip)
    let mut always_0xf0 = [0u8; 1];
    let _ = cursor.read_exact(&mut always_0xf0);

    let mut control = [0u8; 1];
    let _ = cursor.read_exact(&mut control);
    let control_byte = control[0];
    let damage_type = control_byte & 0xF0;

    // always 0x00fe3f
    let mut always_0x00fe3f = [0u8; 3];
    let _ = cursor.read_exact(&mut always_0x00fe3f);

    // killer ID
    let mut killer_id_buf = [0u8; 1];
    let _ = cursor.read_exact(&mut killer_id_buf);
    let killer_id = killer_id_buf[0];

    let mut always_0x000000 = [0u8; 3];
    let _ = cursor.read_exact(&mut always_0x000000);

    // killer Vehicle id with len prefix
    let mut vehicle_len_buf = [0u8; 1];
    let _ = cursor.read_exact(&mut vehicle_len_buf);
    let vehicle_len = vehicle_len_buf[0] as usize;

    let mut vehicle_buf = vec![0u8; vehicle_len];
    let _ = cursor.read_exact(&mut vehicle_buf);
    let killer_vehicle = String::from_utf8(vehicle_buf).unwrap_or_else(|_| String::new());

    Some(KillInfo {
        timestamp_ticks,
        control: control_byte,
        damage_type,
        killer_id,
        killer_vehicle,
    })
}

/// Structure representing a movement packet (basic position info).
#[derive(Debug, Clone)]
pub struct MovementPacket {
    pub eid: u16,
    pub time_ticks: u32,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// Parses the payload of movement (MPI) packets. Only works for server.
pub fn parse_movement_packet(payload_content: &[u8], time_ticks: u32) -> Option<MovementPacket> {
    if payload_content.len() < 40 {
        return None;
    }

    if payload_content.get(0)? != &0xff
        || payload_content.get(1)? != &0x0f
        || payload_content.get(5)? != &0xa3
        || payload_content.get(6)? != &0xf0
        || payload_content.get(10)? != &0x00
        || payload_content.get(11)? != &0x00
        || payload_content.get(13)? != &0x14
    {
        return None;
    }

    // entity ID from sig
    let eid: u16 = (payload_content[2] as u16) | ((payload_content[3] as u16) << 8);

    // offsets 14,22,30
    let get_f64 = |offset: usize| -> Option<f64> {
        let end = offset + 8;
        if end <= payload_content.len() {
            let bytes = &payload_content[offset..end];
            Some(f64::from_le_bytes([
                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
            ]))
        } else {
            None
        }
    };

    let x = get_f64(14)?;
    let y = get_f64(22)?;
    let z = get_f64(30)?;

    // info!(
    //     "Movement packet {{ eid: {}, time_ticks: {}, x: {}, y: {}, z: {} }}",
    //     eid, time_ticks, x, y, z
    // );

    Some(MovementPacket {
        eid,
        time_ticks,
        x,
        y,
        z,
    })
}
