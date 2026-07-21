use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    osanwe::cli::run(osanwe::cli::Cli::parse()).await
}
