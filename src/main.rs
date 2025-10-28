use crate::config::must_get_basic_config;
use crate::router::new_router;
use crate::state::get_app_state;
use axum::BoxError;
use axum::Router;
use axum::error_handling::HandleErrorLayer;
use axum::http::{Method, Uri};
use axum::middleware::from_fn_with_state;
use std::env;
use std::net::SocketAddr;
use std::str::FromStr;
use tibba_hook::{run_after_tasks, run_before_tasks};
use tibba_middleware::{entry, processing_limit, stats};
use tibba_scheduler::run_scheduler_jobs;
use tibba_util::is_development;
use tokio::signal;
use tower::ServiceBuilder;
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::{NotForContentType, Predicate, SizeAbove};
use tracing::{Level, error, info};
use tracing_subscriber::FmtSubscriber;

mod config;
mod dal;
mod image;
mod router;
mod state;

pub async fn handle_error(
    method: Method, // HTTP method of the request
    uri: Uri,       // URI of the request
    err: BoxError,  // The error that occurred
) -> tibba_error::Error {
    // Log the error with request details
    error!("method:{}, uri:{}, error:{}", method, uri, err.to_string());

    // Special handling for timeout errors
    // Otherwise treats as internal server error (500)
    let (message, category, status) = if err.is::<tower::timeout::error::Elapsed>() {
        (
            "Request took too long".to_string(),
            "timeout".to_string(),
            408,
        )
    } else {
        (err.to_string(), "exception".to_string(), 500)
    };

    // Create and return appropriate HttpError
    tibba_error::Error {
        message,
        category,
        status,
        ..Default::default()
    }
}

fn init_logger() {
    let mut level = Level::INFO;
    if let Ok(log_level) = env::var("RUST_LOG")
        && let Ok(value) = Level::from_str(log_level.as_str())
    {
        level = value;
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
        .with_ansi(is_development())
        .finish();
    tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        // TODO 后续有需要可在此设置ping的状态
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
    info!("signal received, starting graceful shutdown");
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    run_before_tasks().await?;
    run_scheduler_jobs().await?;

    // config is validated in init function
    let basic_config = must_get_basic_config();
    let app = if let Some(prefix) = &basic_config.prefix {
        Router::new().nest(prefix, new_router()?)
    } else {
        new_router()?
    };

    let predicate = SizeAbove::new(1024)
        .and(NotForContentType::GRPC)
        .and(NotForContentType::IMAGES)
        .and(NotForContentType::SSE);
    let state = get_app_state();
    let app = app.layer(
        // service build layer execute by add order
        ServiceBuilder::new()
            .layer(HandleErrorLayer::new(handle_error))
            .layer(CompressionLayer::new().compress_when(predicate))
            .timeout(basic_config.timeout)
            .layer(from_fn_with_state(state, entry))
            .layer(from_fn_with_state(state, stats))
            .layer(from_fn_with_state(state, processing_limit)),
    );
    state.run();

    info!(config = ?basic_config, "server is listening");
    let listener = tokio::net::TcpListener::bind(basic_config.listen.clone())
        .await
        .unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap();
    Ok(())
}

async fn start() {
    // only use unwrap in run function
    if let Err(e) = run().await {
        error!(category = "launch_app", message = e.to_string())
    }
    if let Err(e) = run_after_tasks().await {
        error!(category = "run_after_tasks", message = e.to_string(),);
    }
}

fn main() {
    std::panic::set_hook(Box::new(|e| {
        // TODO send alert
        error!(category = "panic", message = e.to_string(),);
        std::process::exit(1);
    }));
    init_logger();
    let cpus = std::env::var("IMAGE_OPTIM_THREADS")
        .map(|v| v.parse::<usize>().unwrap_or(num_cpus::get()))
        .unwrap_or(num_cpus::get())
        .max(1);
    info!(threads = cpus, "start image optim server");
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(cpus)
        .build()
        .unwrap()
        .block_on(start());
}
