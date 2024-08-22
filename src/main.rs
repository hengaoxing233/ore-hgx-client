use clap::{Parser, Subcommand};
use mine::MineArgs;

mod mine;
mod log;

// --------------------------------

/// A command line interface tool for pooling power to submit hashes for proportional ORE rewards
#[derive(Parser, Debug)]
#[command(version, author, about, long_about = None)]
struct Args {
    #[arg(long,
        value_name = "SERVER_URL",
        help = "URL of the server to connect to",
        default_value = "domainexpansion.tech",
    )]
    url: String,
    #[command(subcommand)]
    command: Commands
}

#[derive(Debug, Subcommand)]
enum Commands {
    #[command(about = "Connect to pool and start mining.")]
    Mine(MineArgs),
}

// --------------------------------


#[tokio::main]
async fn main() {
    let args = Args::parse();

    let base_url = args.url;
    match args.command {
        Commands::Mine(args) => {
            mine::mine(args, base_url).await;
        },
    }


}

