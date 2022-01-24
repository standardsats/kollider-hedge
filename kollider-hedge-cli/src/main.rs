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
    #[clap(long)]
    pub sats: i64,
    /// Current exchange rate of the HTLC sats/USD, cannot be specified alongside with price option.
    #[clap(long)]
    pub rate: Option<u64>,
    /// Current exchange rate of the HTLC USD/BTC, cannot be specified alongside with rate option.
    #[clap(long)]
    pub price: Option<f64>,
}

impl HtlcCmd {
    fn rate(&self) -> u64 {
        match (self.rate, self.price) {
            (Some(r), None) => r,
            (None, Some(p)) => (100_000_000. / (p as f64)).round() as u64,
            _ => panic!("Specify only rate or only price parameter!"),
        }
    }
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
        SubCommand::Htlc(cmd) => {
            let rate = cmd.rate();
            client
                .hedge_htlc(HtlcInfo {
                    channel_id: cmd.channel_id,
                    sats: cmd.sats,
                    rate,
                })
                .await?;
            println!("Done");
        }
    }
    Ok(())
}
