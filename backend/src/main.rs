#[tokio::main]
async fn main() {
    if let Err(error) = subboost::cli::run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}
