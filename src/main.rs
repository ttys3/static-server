use std::{net::SocketAddr, sync::Arc};
use tower_http::trace::TraceLayer;

// use axum::{extract::Path as extractPath};

use crate::ResponseError::{BadRequest, FileNotFound, InternalError};
use askama::Template;

use axum::{
    body::{Body, BoxBody},
    extract::Extension,
    http::{header, HeaderValue, Request, Response, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};

use std::path::{Path, PathBuf};
use tower::util::ServiceExt;
use tower_http::services::ServeDir;

use std::ffi::OsStr;
use std::io;
use tokio::fs;

use clap::Parser;

// for IpAddr::from_str
use axum::extract::ConnectInfo;
use percent_encoding::percent_decode;
use std::str::FromStr;

#[derive(Parser, Debug)]
#[clap(name = "static-server", about = "A simple static file server written in Rust based on axum framework.")]
struct Opt {
    /// set the log level
    #[clap(short = 'l', long = "log", default_value = "debug")]
    log_level: String,

    /// set the root directory
    #[clap(short = 'r', long = "root", default_value = ".")]
    root_dir: String,

    /// set the listen addr
    #[clap(short = 'a', long = "addr", default_value = "127.0.0.1")]
    addr: String,

    /// set the listen port
    #[clap(short = 'p', long = "port", default_value = "3000")]
    port: u16,
}

struct StaticServerConfig {
    pub(crate) root_dir: String,
}

#[tokio::main]
async fn main() {
    let opt = Opt::parse();
    tracing::debug!("opt={:#?}", opt);

    // Set the RUST_LOG, if it hasn't been explicitly defined
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", format!("static_server={},tower_http={}", opt.log_level, opt.log_level))
    }
    tracing_subscriber::fmt::init();

    // strip "/" suffix from roor dir so that we can strip prefix safely (to ensure we get absolute uri path)
    // root_dir = "/foo/bar/", prefix = "/foo/bar", real path = "/foo/bar/sub1/file1.txt", result uri = "/sub1/file1.txt"
    // but "/" is still "/", so we need to handle it specially when strip prefix
    // so "./" -> "."
    // "/foo/" -> "/foo"
    // "" or "." equal to current directory
    let mut root_dir = opt.root_dir;
    if root_dir != "/" {
        root_dir = root_dir.trim_end_matches('/').to_string();
    }

    let app = Router::new()
        .route("/favicon.ico", get(favicon))
        .route("/healthz", get(health_check))
        .fallback(get(index_or_content))
        .layer(Extension(Arc::new(StaticServerConfig { root_dir })))
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
            let ConnectInfo(addr) = request.extensions().get::<ConnectInfo<SocketAddr>>().unwrap();
            let empty_val = &HeaderValue::from_static("");
            let user_agent = request.headers().get("User-Agent").unwrap_or(empty_val).to_str().unwrap_or("");
            tracing::debug_span!("client-addr", addr = %addr, user_agent=%user_agent)
        }));

    let addr = std::net::IpAddr::from_str(opt.addr.as_str()).unwrap_or_else(|_| "127.0.0.1".parse().unwrap());

    let sock_addr = SocketAddr::from((addr, opt.port));

    tracing::info!("listening on http://{}", sock_addr);

    axum::Server::bind(&sock_addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .unwrap();
}

// see https://kubernetes.io/docs/reference/using-api/health-checks/
async fn health_check() -> impl IntoResponse {
    "ok"
}

async fn favicon() -> impl IntoResponse {
    // one pixel favicon generated from https://png-pixel.com/
    let one_pixel_favicon = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mPk+89QDwADvgGOSHzRgAAAAABJRU5ErkJggg==";
    let pixel_favicon = base64::decode(one_pixel_favicon).unwrap();
    ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], pixel_favicon)
}

// Request<Body> used an extractors cannot be combined with other unless Request<Body> is the very last extractor.
// see https://docs.rs/axum/latest/axum/extract/index.html#applying-multiple-extractors
// see https://github.com/tokio-rs/axum/discussions/583#discussioncomment-1739582
async fn index_or_content(Extension(cfg): Extension<Arc<StaticServerConfig>>, req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    return match ServeDir::new(&cfg.root_dir).oneshot(req).await {
        Ok(res) => match res.status() {
            StatusCode::NOT_FOUND => {
                let path = path.trim_start_matches('/');
                let path = percent_decode(path.as_ref()).decode_utf8_lossy();

                let mut full_path = PathBuf::new();
                full_path.push(&cfg.root_dir);
                for seg in path.split('/') {
                    if seg.starts_with("..") || seg.contains('\\') {
                        return Err(ErrorTemplate {
                            err: BadRequest("invalid path".to_string()),
                            cur_path: path.to_string(),
                            message: "invalid path".to_owned(),
                        });
                    }
                    full_path.push(seg);
                }

                let cur_path = Path::new(&full_path);

                match cur_path.is_dir() {
                    true => {
                        let rs = visit_dir_one_level(&full_path, &cfg.root_dir).await;
                        match rs {
                            Ok(files) => Ok(DirListTemplate {
                                lister: DirLister { files },
                                cur_path: path.to_string(),
                            }
                            .into_response()),
                            Err(e) => Err(ErrorTemplate {
                                err: InternalError(e.to_string()),
                                cur_path: path.to_string(),
                                message: e.to_string(),
                            }),
                        }
                    }
                    false => Err(ErrorTemplate {
                        err: FileNotFound("file not found".to_string()),
                        cur_path: path.to_string(),
                        message: "file not found".to_owned(),
                    }),
                }
            }
            _ => Ok(res.map(axum::body::boxed)),
        },
        Err(err) => Err(ErrorTemplate {
            err: InternalError(format!("Unhandled error: {}", err)),
            cur_path: path.to_string(),
            message: format!("Unhandled error: {}", err),
        }),
    };
}

// io::Result<Vec<DirEntry>>
async fn visit_dir_one_level(path: &Path, prefix: &str) -> io::Result<Vec<FileInfo>> {
    let mut dir = fs::read_dir(path).await?;
    // let mut files = Vec::new();
    let mut files: Vec<FileInfo> = Vec::new();

    while let Some(child) = dir.next_entry().await? {
        // files.push(child)

        let the_path = child.path().to_string_lossy().to_string();
        let the_uri_path: String;
        if !prefix.is_empty() && !the_path.starts_with(prefix) {
            tracing::error!("visit_dir_one_level skip invalid path={}", the_path);
            continue;
        } else if prefix != "/" {
            the_uri_path = the_path.strip_prefix(prefix).unwrap().to_string();
        } else {
            the_uri_path = the_path;
        }
        files.push(FileInfo {
            name: child.file_name().to_string_lossy().to_string(),
            ext: Path::new(child.file_name().to_str().unwrap())
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string(),
            // path: the_path,
            path_uri: the_uri_path,
            is_file: child.file_type().await?.is_file(),
            last_modified: child
                .metadata()
                .await?
                .modified()?
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64,
        });
    }

    Ok(files)
}

mod filters {
    pub(crate) fn datetime(ts: &i64) -> ::askama::Result<String> {
        if let Ok(format) = time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second] UTC") {
            return Ok(time::OffsetDateTime::from_unix_timestamp(*ts).unwrap().format(&format).unwrap());
        }
        Err(askama::Error::Fmt(std::fmt::Error))
    }
}

#[derive(Template)]
#[template(path = "index.html")]
struct DirListTemplate {
    lister: DirLister,
    cur_path: String,
}

#[derive(Template)]
#[template(path = "error.html")]
struct ErrorTemplate {
    err: ResponseError,
    cur_path: String,
    message: String,
}

impl IntoResponse for ErrorTemplate {
    fn into_response(self) -> Response<BoxBody> {
        let t = self;
        match t.render() {
            Ok(html) => {
                let mut resp = Html(html).into_response();
                match t.err {
                    ResponseError::FileNotFound(_) => {
                        *resp.status_mut() = StatusCode::NOT_FOUND;
                    }
                    ResponseError::BadRequest(_) => {
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                    }
                    ResponseError::InternalError(_) => {
                        *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                    }
                }
                resp
            }
            Err(err) => {
                tracing::error!("template render failed, err={}", err);
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to render template. Error: {}", err)).into_response()
            }
        }
    }
}

enum ResponseError {
    BadRequest(String),
    FileNotFound(String),
    InternalError(String),
}

struct DirLister {
    files: Vec<FileInfo>,
}

struct FileInfo {
    name: String,
    ext: String,
    // path: String,
    path_uri: String,
    is_file: bool,
    last_modified: i64,
}

impl IntoResponse for DirListTemplate {
    fn into_response(self) -> Response<BoxBody> {
        let t = self;
        match t.render() {
            Ok(html) => Html(html).into_response(),
            Err(err) => {
                tracing::error!("template render failed, err={}", err);
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to render template. Error: {}", err)).into_response()
            }
        }
    }
}
