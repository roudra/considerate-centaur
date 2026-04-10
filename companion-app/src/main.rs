use tracing_subscriber::EnvFilter;

mod learner;
mod assignments;
mod progress;
mod session;
mod claude;
mod dashboard;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    tracing::info!("Educational Companion starting up");

    let app = axum::Router::new()
        .route("/health", axum::routing::get(|| async { "ok" }));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000")
        .await
        .expect("Failed to bind to port 3000");

    tracing::info!("Listening on http://0.0.0.0:3000");

    axum::serve(listener, app)
        .await
        .expect("Server error");
}
