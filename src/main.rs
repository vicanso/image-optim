use axum::{error_handling::HandleErrorLayer, middleware::from_fn, routing::get, Router};
use std::time::Duration;
use std::{env, net::SocketAddr, str::FromStr};
use tokio::signal;
use tower::ServiceBuilder;
use tracing::Level;
use tracing_subscriber::FmtSubscriber;

mod error;
mod image_processing;
mod images;
mod middleware;
mod optim;
mod response;

mod task_local;

fn init_logger() {
    let mut level = Level::INFO;
    if let Ok(log_level) = env::var("LOG_LEVEL") {
        if let Ok(value) = Level::from_str(log_level.as_str()) {
            level = value;
        }
    }
    let timer = tracing_subscriber::fmt::time::OffsetTime::local_rfc_3339().unwrap_or_else(|_| {
        tracing_subscriber::fmt::time::OffsetTime::new(
            time::UtcOffset::from_hms(0, 0, 0).unwrap(),
            time::format_description::well_known::Rfc3339,
        )
    });

    let subscriber = FmtSubscriber::builder()
        .with_max_level(level)
        .with_timer(timer)
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

#[tokio::main]
async fn run() {
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
                .layer(HandleErrorLayer::new(error::handle_error))
                .timeout(Duration::from_secs(30)),
        )
        // 后面的layer先执行
        .layer(from_fn(middleware::access_log))
        .layer(from_fn(middleware::entry));

    let port = 3000;
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!(port, "Server is starting");

    axum::Server::bind(&addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .unwrap();
}

async fn ping() -> &'static str {
    "pong"
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

fn main() {
    // Because we need to get the local offset before Tokio spawns any threads, our `main`
    // function cannot use `tokio::main`.

    init_logger();
    run();
}
