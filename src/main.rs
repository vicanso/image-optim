use axum::{error_handling::HandleErrorLayer, routing::get, BoxError, Router};
use error::HTTPError;
use std::time::Duration;
use std::{env, net::SocketAddr, str::FromStr};
use tower::ServiceBuilder;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;
use tokio::signal;

mod error;
mod image_processing;
mod images;
mod optim;
mod response;

fn init_logger() {
    let mut level = Level::INFO;
    if let Ok(log_level) = env::var("LOG_LEVEL") {
        if let Ok(value) = Level::from_str(log_level.as_str()) {
            level = value;
        }
    }
    let subscriber = FmtSubscriber::builder().with_max_level(level).finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

#[tokio::main]
async fn main() {
    init_logger();

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // TODO 是否记录异常
        tracing::info!("panic info:{:?}", info);
        default_panic(info);
    }));
    let app = Router::new()
        .route("/ping", get(ping))
        .merge(optim::new_router())
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(handle_error))
                .timeout(Duration::from_secs(30)),
        );

    let port = 3000;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(port, "Server is starting");

    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn ping() -> &'static str {
    "pong"
}

async fn handle_error(err: BoxError) -> HTTPError {
    if err.is::<tower::timeout::error::Elapsed>() {
        HTTPError {
            message: "Request took too long".to_string(),
            category: "timeout".to_string(),
            status: 408,
        }
    } else {
        HTTPError {
            message: format!("Unhandled internal error: {}", err),
            category: "internalServerError".to_string(),
            status: 500,
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("signal received, starting graceful shutdown");
}