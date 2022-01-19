use clap::Parser;
use std::error::Error;

use kollider_hedge_client::client::HedgeClient;
use kollider_hedge_domain::api::HtlcInfo;

#[derive(Parser, Debug)]
#[clap(about, version, author)]
struct Args {
    #[clap(long, default_value = "http://127.0.0.1:8081")]
    url: String,
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Parser, Debug)]
enum SubCommand {
    /// Query current state of the hedge service
    State,
    /// Add or remove sats from hedge position
    Htlc(HtlcCmd),
}

#[derive(Parser, Debug)]
struct HtlcCmd {
    /// ID of channel
    pub channel_id: String,
    /// Amount of satoshis, negative number represents withdraw
    pub sats: i64,
    /// Current exchange rate of the HTLC sats/usd
    pub rate: u64,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    env_logger::init();
    let client = HedgeClient::new(&args.url);

    match args.subcmd {
        SubCommand::State => {
            let state = client.query_state().await?;
            let pretty = serde_json::to_string_pretty(&state)?;
            println!("{}", pretty);
        }
        SubCommand::Htlc(HtlcCmd {
            channel_id,
            sats,
            rate,
        }) => {
            client
                .hedge_htlc(HtlcInfo {
                    channel_id,
                    sats,
                    rate,
                })
                .await?;
            println!("Done");
        }
    }
    Ok(())
}
