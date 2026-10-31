use std::error::Error;
use std::io;

pub fn logging() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")))
        .without_time()
        .init();
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler").recv().await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

pub fn connection_error(error: &hyper::Error) {
    let mut source: Option<&dyn Error> = Some(error);
    while let Some(current) = source {
        if let Some(io) = current.downcast_ref::<io::Error>()
            && matches!(io.kind(), io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted | io::ErrorKind::BrokenPipe)
        {
            tracing::debug!(%error, "HTTP peer disconnected");
            return;
        }
        source = current.source();
    }
    tracing::warn!(%error, "HTTP connection failed");
}
