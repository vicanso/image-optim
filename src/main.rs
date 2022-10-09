use axum::{routing::get, Router};
use std::net::SocketAddr;

mod error;
mod image;
mod optim;
mod response;

#[tokio::main]
async fn main() {
    let app = Router::new()
        .route("/ping", get(ping))
        .merge(optim::new_router());

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

async fn ping() -> &'static str {
    "pong"
}
