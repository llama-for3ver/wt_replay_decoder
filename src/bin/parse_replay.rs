use clap::{crate_authors, Parser};
use log::{debug, error, info, warn};
use std::fs::{self};
use std::path::PathBuf;
use std::process::exit;
use wrpl::{header, parser, utils};

#[derive(Parser, Debug)]
#[command(
    author = crate_authors!(),
    version = "0.6",
    about = "A CLI utility to parse replay files, extracting header, chat messages, and end-of-replay results.
Designed only for client replays, chat message parsing will fail otherwise.",
help_template = "\
{name} {version} ({author})
{about}

USAGE:
    {usage}

EXAMPLES:
    ./parse_replay -r ./#2025.05.05.wrpl
    ./parse_replay -r ./#2025.05.05.wrpl --skip-zlib --offset 0x000004D1

OPTIONS:
    {options}
"
)]
struct Args {
    /// Path to the replay file to decode.
    #[arg(short, long)]
    replay_file: PathBuf,

    /// Manually specify offset where the data stream starts (zlib or raw).
    /// Overrides automatic detection. Use hex (0x...) or decimal.
    #[arg(short, long, value_parser = utils::parse_offset)] // Using utils
    offset: Option<u64>,

    /// Skip zlib decompression and parse raw packet data [directly from the offset].
    #[arg(long, default_value_t = false)]
    skip_zlib: bool,

    /// Parse replay results data (requires header parsing).
    #[arg(long, default_value_t = false)]
    parse_results: bool,
}

fn humanize_victory_or_loss(input: &str) -> String {
    match input {
        "fail" => "Victory".to_string(),
        "success" => "Defeat".to_string(),
        // not sure what this one means
        "left" => "Draw".to_string(),
        _ => "Unknown".to_string(),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    // read the file into memory first
    let file_data = match fs::read(&args.replay_file) {
        Ok(data) => data,
        Err(e) => {
            error!("Error reading replay file {:?}: {}", args.replay_file, e);
            exit(1);
        }
    };

    let start_offset: u64;
    let mut has_wrpl_header = false;

    // check for magic bytes
    // FIXME: magic bytes are actually longer than 2 bytes
    if file_data.len() >= 2 {
        if file_data[0..2] == *b"\xE5\xAC" {
            has_wrpl_header = true;
            debug!("File starts with magic bytes, assuming normal .wrpl.");
        } else {
            warn!(
                "File does not start with magic bytes.
                Assuming it contains only stream data"
            );
        }
    } else {
        error!("File is too short!");
    }

    let header_info = if has_wrpl_header {
        // parse the header (still needs path for now)
        // which is suboptimal!!
        match header::parse_header(&file_data) {
            Ok(header) => {
                info!("Successfully parsed replay header:");
                println!("{}", header);
                Some(header)
            }
            Err(e) => {
                error!("Failed to parse replay header: {}", e);
                None
            }
        }
    } else {
        None
    };

    if let Some(user_offset) = args.offset {
        info!(
            "Using provided offset: {:#0x} ({})",
            user_offset, user_offset
        );
        if has_wrpl_header {
            debug!(
                "Ignoring header parsing and zlib search
            because offset was provided."
            );
        } else {
            warn!(
                "--offset provided, but file does not start
                with known header.
                Stream may start at 0 (unless you are sure)."
            );
        }
        start_offset = user_offset;
    } else if has_wrpl_header && header_info.is_some() {
        if args.skip_zlib {
            warn!("--skip-zlib provided, but file appears to be a standard .wrpl (starts with E5 AC).");
            info!(
                "Assuming raw stream starts at offset 0 (header will be skipped).
            Consider using --offset if data is after header."
            );
            start_offset = 0; // default to 0 if skipping zlib
                              // even with wrpl header, user might want to skip header
        } else {
            info!("Attempting to auto-detect zlib stream start offset (searching after 0xE5AC)...");
            // Use utils::find_zlib_header_offset, but it needs a Path. Pass the path.
            // If it were refactored to take &[u8], we'd pass &file_data.
            // also we should NOT search after 0xE5AC, but after the real header
            match utils::find_zlib_header_offset(&args.replay_file, 2, None) {
                Ok(Some(detected_offset)) => {
                    start_offset = detected_offset;
                }
                Ok(None) => {
                    error!("Failed to automatically find zlib stream start.");
                    eprintln!("You may need to specify the offset manually using --offset.");
                    exit(1);
                }
                Err(e) => {
                    error!("Error during zlib header search: {:?}", e);
                    exit(1);
                }
            }
        }
    } else {
        // no WRPL header detected and no offset provided.
        info!("Assuming stream starts at offset 0.");
        if !args.skip_zlib {
            warn!(
                "File does not look like a .wrpl and --skip-zlib not specified.
                Will attempt zlib decompression from offset 0, but this may not work."
            );
        }
        start_offset = 0;
    }

    let replay_result = if args.parse_results && header_info.is_some() {
        parser::process_replay_stream(
            &file_data,
            start_offset,
            args.skip_zlib,
            Some(header_info.as_ref().unwrap()),
        )
    } else {
        parser::process_replay_stream(&file_data, start_offset, args.skip_zlib, None)
    };

    match replay_result {
        Ok(stats) => {
            // display chat messages here...
            // omit channel_type and is_enemy for now
            if !stats.chat_messages.is_empty() {
                info!("Found {} chat messages:", stats.chat_messages.len());
                for (i, chat) in stats.chat_messages.iter().enumerate() {
                    info!(
                        "{}: {} says '{}' ", // (channel: {:?}, enemy: {:?})",
                        i + 1,
                        chat.sender,
                        chat.message,
                        // chat.channel_type,
                        // chat.is_enemy
                    );
                }
            }

            // try and display replay results
            if let Some(ref results) = stats.replay_results {
                info!("Found {} players", results.players.len());
                info!("Status: {}", humanize_victory_or_loss(&results.status));
                info!("Time Played: {:.1} seconds", results.time_played);
                info!("Author: {} [{}]", results.author, results.author_user_id);
            } else if args.parse_results {
                warn!("Replay results parsing was requested but no results found");
            }

            debug!("Processing Stats:");
            debug!("  Packets Processed: {}", stats.packet_count);
            debug!(
                "  Total Decompressed Bytes: {}",
                stats.total_decompressed_bytes
            );

            // if stats.final_offset > 0 {
            //     info!("  Final Offset: {:#0x}", stats.final_offset);
            // } else if !args.skip_zlib {
            //     info!("  Final Offset: Unknown (due to zlib stream)");
            // }
        }
        Err(e) => {
            error!("Error during replay stream processing: {:?}", e);
            exit(1);
        }
    }

    info!("Successfully finished processing!");
}
