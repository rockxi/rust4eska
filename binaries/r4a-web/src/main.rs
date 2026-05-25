use axum::{
    body::Body,
    http::{header, Response, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use clap::Parser;
use rust_embed::RustEmbed;
use tracing::info;

#[derive(RustEmbed)]
#[folder = "dist/"]
struct Assets;

#[derive(Parser)]
#[command(name = "r4a-web", about = "r4a Web Interface")]
struct Cli {
    #[arg(long, default_value = "8081")]
    port: u16,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .fallback(static_handler);

    let addr = format!("0.0.0.0:{}", cli.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("r4a-web listening on http://{}", addr);

    axum::serve(listener, app).await?;
    Ok(())
}

async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    if path.is_empty() || path == "index.html" {
        return index_html();
    }

    match Assets::get(path) {
        Some(content) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(Body::from(content.data))
                .unwrap()
        }
        None => {
            index_html()
        }
    }
}

fn index_html() -> Response<Body> {
    match Assets::get("index.html") {
        Some(content) => Response::builder()
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from(content.data))
            .unwrap(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("404 Not Found"))
            .unwrap(),
    }
}
