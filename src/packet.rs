use anyhow::Context;
use log::{debug, error, warn};
use std::io::{self, Cursor, Read};

use anyhow::{bail, Result};

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

/// Parses the payload of a chat packet. This is type `4`.
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
