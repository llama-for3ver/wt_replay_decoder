use std::fs;
use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[clap(
    author = "llama-for3ver",
    about = "A CLI util to download all associated replays for a given game ID.",
    version = "1.0",
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
    /// The replay ID in hex format.
    #[clap(short, long)]
    id: String,

    /// The directory to save the replays to. Default "replays".
    #[clap(short, long, default_value = "replays")]
    output_dir: String,
}

#[tokio::main]
/// Downloads all replays associated with a given ID.
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let output_dir = PathBuf::from(&args.output_dir);

    // create the output directory if it doesn't exist...
    fs::create_dir_all(&output_dir)?;

    println!("Downloading replays for ID: {}", args.id);
    println!("Saving replays to: {}", output_dir.display());

    let mut i = 0;
    loop {
        let url = format!(
            // seems to need a 0 prepended
            // also this specific domain is optimal -
            // the something.warthunder.com one has rate limiting
            // but still redirects to this one
            "https://wt-replays-cdnnow.cdn.gaijin.net/0{}/{:04}.wrpl",
            args.id, i
        );

        let response = reqwest::get(&url).await?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            println!("Replay {} not found, stopping.", i);
            break;
        }

        if !response.status().is_success() {
            eprintln!("Error downloading replay {}: {}", i, response.status());
            break;
        }

        let file_path = output_dir.join(format!("{:04}.wrpl", i));
        let mut file = fs::File::create(&file_path)?;
        let content = response.bytes().await?;
        std::io::copy(&mut content.as_ref(), &mut file)?;

        println!("Downloaded replay: {:04}.wrpl", i);
        i += 1;
    }
    Ok(())
}
