use clap::Parser;
use flate2::read::ZlibDecoder;
use log::{error, info, warn};
use std::fs;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use wrpl::utils;

#[derive(Parser, Debug)]
#[command(
    author = "llama-for3ver",
    version = "0.1",
    about = "Utility to find the stream offset. Purely for debugging purposes.",
    help_template = "\
{name} {version} ({author})
{about}

USAGE:
    {usage}

OPTIONS:
    {options}
"
)]
struct Args {
    /// Path to the file file
    #[arg(short, long)]
    replay_file: PathBuf,

    /// Scan without Zlib
    #[arg(long, default_value_t = false)]
    skip_zlib: bool,

    /// Range to search start, default 0.
    #[arg(long, default_value = "0", value_parser = utils::parse_offset)]
    search_start: u64,

    /// Range to search stop (not inclusive), default file end.
    #[arg(long, default_value = "0", value_parser = utils::parse_offset)]
    search_end: u64,
}

// Structure to hold both the chat info and the raw packet as hex
#[derive(Debug)]
struct BruteResult {
    offset: u64,
    chat_desc: Option<String>,
    packet_hex: String,
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    let data = match fs::read(&args.replay_file) {
        Ok(d) => d,
        Err(e) => {
            error!("Couldn't read replay file: {:?}", e);
            std::process::exit(1);
        }
    };

    let file_len = data.len() as u64;
    let search_start = args.search_start;
    let search_end = if args.search_end == 0 {
        file_len
    } else {
        args.search_end
    };

    if search_start >= search_end || search_end > file_len {
        error!("Invalid search range: {} .. {}", search_start, search_end);
        std::process::exit(1);
    }

    info!(
        "Searching offsets {} to {} ({:#x} .. {:#x}) in {:?}",
        search_start, search_end, search_start, search_end, args.replay_file
    );

    let mut candidates = Vec::new();

    let brute_step = 1u64; // 1, this is for performance

    for offset in (search_start..search_end).step_by(brute_step as usize) {
        if let Some((chat_info, packet_hex)) =
            try_parse_chat_at_offset(&data, offset, args.skip_zlib)
        {
            info!(
                "Potential chat at offset {:#x} ({}) - sender: '{}', msg: '{}'",
                offset, offset, chat_info.sender, chat_info.message
            );
            candidates.push(BruteResult {
                offset,
                chat_desc: Some(format!("'{}': '{}'", chat_info.sender, chat_info.message)),
                packet_hex,
            });
        }
    }

    if candidates.is_empty() {
        warn!(
            "Couldn't find anything with offsets {:#x} to {:#x} :(",
            search_start, search_end
        );
    } else {
        println!("\nSearch complete - top candidates:");
        for cand in candidates.iter() {
            println!(
                "  Offset {:#08x}  | {}\n    Raw chat packet: {}",
                cand.offset,
                cand.chat_desc.as_ref().unwrap(),
                cand.packet_hex
            );
        }
        println!(
            "\nFirst likely stream offset: {:#x} ({})",
            candidates[0].offset, candidates[0].offset
        );
    }
}

fn try_parse_chat_at_offset(
    data: &[u8],
    offset: u64,
    skip_zlib: bool,
) -> Option<(wrpl::parser::ChatInfo, String)> {
    if offset as usize >= data.len() - 10 {
        return None;
    }

    // open decompression if needed
    let tail = &data[offset as usize..];
    let mut reader: Box<dyn Read> = if skip_zlib {
        Box::new(Cursor::new(tail))
    } else {
        Box::new(ZlibDecoder::new(Cursor::new(tail)))
    };

    let mut last_timestamp = 0u32;
    for _ in 0..6 {
        let size_prefix = match wrpl::parser::read_variable_length_size(&mut reader) {
            Ok(Some((packet_size, _))) if packet_size > 0 && packet_size < 0x2000 => packet_size,
            _ => return None,
        };
        let mut packet_buf = vec![0u8; size_prefix as usize];
        if let Err(_) = reader.read_exact(&mut packet_buf) {
            return None;
        }
        let parsed_header = wrpl::parser::read_packet_header_from_stream(
            &mut Cursor::new(&packet_buf),
            last_timestamp,
        );
        let (ptype, timestamp, header_sz) = match parsed_header {
            Ok(Some((ptype, timestamp, header_sz))) => (ptype, timestamp, header_sz),
            _ => return None,
        };

        last_timestamp = timestamp;

        if ptype == 3 {
            let payload = &packet_buf[header_sz..];
            let chat = wrpl::parser::parse_chat_packet(payload, timestamp);
            if let Some(chat_info) = chat {
                // sanity checks
                if !chat_info.sender.is_empty()
                    && !chat_info.message.is_empty()
                    && chat_info.sender.chars().all(|c| c.is_ascii_graphic())
                    && chat_info.message.chars().any(|c| c.is_ascii_alphanumeric())
                    && chat_info.message.len() <= 128
                    && chat_info.sender.len() <= 32
                {
                    // Show the *raw* chat packet as hex, including header + payload
                    return Some((chat_info, wrpl::utils::hex::encode(&packet_buf)));
                }
            }
        }
    }
    None
}
