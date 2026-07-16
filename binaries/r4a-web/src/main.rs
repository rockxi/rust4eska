use anyhow::Result;
use axum::{
    body::Body,
    http::{header, Response, StatusCode, Uri},
    response::IntoResponse,
    routing::get,
    Router,
};
use clap::{Parser, Subcommand};
use rust_embed::RustEmbed;
use tracing::info;

#[derive(RustEmbed)]
#[folder = "dist/"]
struct Assets;

#[derive(Parser)]
#[command(name = "r4a-web", about = "r4a Web Interface")]
struct Cli {
    #[command(subcommand)]
    command: Option<Cmd>,
    #[arg(long, default_value = "3502")]
    port: u16,
}

#[derive(Subcommand)]
enum Cmd {
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    Enable {
        #[arg(long, default_value = "3502")]
        port: u16,
    },
    Disable,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Some(Cmd::Service { action }) => handle_service(action),
        None => serve(cli.port).await,
    }
}

fn handle_service(action: ServiceAction) -> Result<()> {
    let manager = r4a_service::ServiceManager::detect()?;
    match action {
        ServiceAction::Enable { port } => {
            let exec = format!("/usr/local/bin/r4a-web --port {}", port);
            manager.enable("r4a-web", "r4a Web UI", &exec, &[])?;
        }
        ServiceAction::Disable => {
            manager.disable("r4a-web")?;
        }
    }
    Ok(())
}

async fn serve(port: u16) -> Result<()> {
    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .fallback(static_handler);

    let addr = format!("0.0.0.0:{}", port);
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
        None => index_html(),
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
