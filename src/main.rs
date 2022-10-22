use axum::{error_handling::HandleErrorLayer, routing::get, BoxError, Router};
use error::HTTPError;
use std::net::SocketAddr;
use std::process;
use std::time::Duration;
use tower::ServiceBuilder;

mod error;
mod image_processing;
mod images;
mod optim;
mod response;

#[tokio::main]
async fn main() {
    ctrlc::set_handler(|| {
        // TODO
        // 退出程序，增加graceful close处理
        process::exit(0);
    })
    .unwrap();

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // TODO 是否记录异常
        println!("{:?}", info);
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

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
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
