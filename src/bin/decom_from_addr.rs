use flate2::read::ZlibDecoder;
use std::env;
use std::fs::File;
use std::io::{self, BufReader, Seek, SeekFrom};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    // FIXME: use clap
    if args.len() != 4 {
        eprintln!("Usage: {} <input_file> <offset> <output_file>", args[0]);
        std::process::exit(1);
    }
    let input_path = &args[1];
    let offset: u64 = args[2].parse().expect("Invalid offset");
    let output_path = &args[3];

    let file = File::open(input_path)?;
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::Start(offset))?;

    let mut decoder = ZlibDecoder::new(reader);

    let mut output = File::create(output_path)?;
    io::copy(&mut decoder, &mut output)?;

    Ok(())
}
