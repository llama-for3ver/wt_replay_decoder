use anyhow::{Context, Result};

use log::{debug, info, warn};
use memchr::memmem;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

// FIXME: we do NOT need to search for a zlib header manually
// but i can't be bothered changing right now

/// this is 64 kB
const ZLIB_SEARCH_CHUNK_SIZE: usize = 64 * 1024;
/// ZLib magic bytes to search for
const ZLIB_HEADERS: [&[u8]; 3] = [
    b"\x78\x5E", // best Speed - currently used (for client-side)
    b"\x78\x9C", // default
    b"\x78\xDA", // best compression
];
/// Max length of headers in ZLIB_HEADERS
const MAX_ZLIB_HEADER_LEN: usize = 2;

/// Helper for parsing user input
pub fn parse_offset(s: &str) -> Result<u64, String> {
    // FIXME: use lower
    if let Some(hex_val) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u64::from_str_radix(hex_val, 16).map_err(|e| e.to_string())
    } else {
        s.parse::<u64>().map_err(|e| e.to_string())
    }
}

/// helper for hex encoding
pub mod hex {
    /// converts a byte slice to a hex string.
    pub fn encode<T: AsRef<[u8]>>(data: T) -> String {
        let bytes = data.as_ref();
        let mut s = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            std::fmt::Write::write_fmt(&mut s, format_args!("{:02x}", byte)).unwrap();
        }
        s
    }
}

/// Searches for common zlib headers in a byte slice.
/// Returns the offset of the first header found, or None if not found.
pub fn find_zlib_header(data: &[u8], search_start_offset: usize) -> Option<usize> {
    let search_data = &data[search_start_offset..];

    // Build finders for efficiency
    let finders: Vec<_> = ZLIB_HEADERS
        .iter()
        .map(|h| memmem::Finder::new(h))
        .collect();

    // Search for headers in the data
    let mut first_found_offset = None;

    for (i, finder) in finders.iter().enumerate() {
        if let Some(idx) = finder.find(search_data) {
            let absolute_offset = search_start_offset + idx;

            match first_found_offset {
                Some(prev_offset) if absolute_offset < prev_offset => {
                    debug!(
                        "Found earlier zlib header {} at offset {:#0x}",
                        hex::encode(ZLIB_HEADERS[i]),
                        absolute_offset
                    );
                    first_found_offset = Some(absolute_offset);
                }
                None => {
                    debug!(
                        "Found zlib header {} at offset {:#0x}",
                        hex::encode(ZLIB_HEADERS[i]),
                        absolute_offset
                    );
                    first_found_offset = Some(absolute_offset);
                }
                _ => {} // found one, but not earlier than the current best
            }
        }
    }

    first_found_offset
}

/// Searches for common zlib headers in a *file* within a range.
/// Returns the offset of the first header found, or None if not found.
pub fn find_zlib_header_offset(
    file_path: &Path,
    search_start_offset: u64,
    search_end_offset: Option<u64>,
) -> Result<Option<u64>> {
    let file = File::open(file_path)
        .with_context(|| format!("Failed to open file for searching: {:?}", file_path))?;
    let mut reader = BufReader::with_capacity(ZLIB_SEARCH_CHUNK_SIZE, file);

    reader
        .seek(SeekFrom::Start(search_start_offset))
        .context("Failed to seek to search start offset")?;

    let mut buffer = Vec::with_capacity(ZLIB_SEARCH_CHUNK_SIZE + MAX_ZLIB_HEADER_LEN);
    let mut current_file_pos = search_start_offset;
    let mut overlap: Vec<u8> = Vec::new(); // Stores the overlap from the previous chunk

    loop {
        // prepare buffer for next read: keep overlap from previous iteration
        buffer.clear();
        buffer.extend_from_slice(&overlap);

        // Read next chunk
        let mut chunk_buf = vec![0; ZLIB_SEARCH_CHUNK_SIZE];
        let bytes_read = reader
            .read(&mut chunk_buf)
            .context("Failed to read chunk during zlib header search")?;

        if bytes_read == 0 {
            debug!("EOF reached during zlib header search.");
            break; // EOF
        }

        buffer.extend_from_slice(&chunk_buf[..bytes_read]);

        // search for headers in the current buffer...
        if let Some(offset) = find_zlib_header(&buffer, 0) {
            let absolute_offset =
                current_file_pos.saturating_sub(overlap.len() as u64) + offset as u64;
            info!(
                "Detected zlib header at offset {:#0x} ({})",
                absolute_offset, absolute_offset
            );
            return Ok(Some(absolute_offset));
        }

        // prepare overlap for next iteration (max header len - 1 bytes)
        let overlap_start = buffer.len().saturating_sub(MAX_ZLIB_HEADER_LEN - 1);
        overlap = buffer[overlap_start..].to_vec();

        // update position for the start of the *next* reads data
        current_file_pos += bytes_read as u64;

        // Check search limit
        // if let Some(limit) = search_end_offset {
        //     if current_file_pos >= limit {
        //         debug!("Search limit reached.");
        //         break;
        //     }
        // }
    }

    // if loop finishes without finding header
    let search_range_str = if let Some(limit) = search_end_offset {
        format!(
            "from offset {:#0x} up to {:#0x}",
            search_start_offset, limit
        )
    } else {
        format!("from offset {:#0x} to end of file", search_start_offset)
    };

    warn!(
        "Could not find any common zlib headers ({:?}) {}",
        ZLIB_HEADERS.iter().map(hex::encode).collect::<Vec<_>>(),
        search_range_str
    );
    warn!("Are you sure this is a complete full client-side replay?");
    Ok(None)
}
