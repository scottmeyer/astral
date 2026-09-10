use clap::Parser;
use ostk_gpt_cache::{
    config::Config,
    proxy::{App, router},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    let address = config.listen;
    let app = App::new(config).await?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    eprintln!(
        "ostk-gpt-cache {} listening on {}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr()?
    );
    axum::serve(listener, router(app))
        .with_graceful_shutdown(shutdown())
        .await?;
    Ok(())
}

async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
