use axum_macros::debug_handler;
use std::collections::HashMap;
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;

use crate::ResponseError::{BadRequest, FileNotFound, InternalError};
use askama::Template;

use axum::{
    body::Body,
    extract::State,
    http::{header, HeaderValue, Request, Response, StatusCode},
    response::{Html, IntoResponse},
    routing::get,
    Router,
};

use std::path::{Path, PathBuf};
use tower::util::ServiceExt;
use tower_http::services::ServeDir;

use std::ffi::OsStr;
use std::process::Stdio;
use tokio::fs;
use tokio::io;

use clap::Parser;

// for IpAddr::from_str
use axum::extract::{connect_info::ConnectInfo, Query};
use axum::routing::get_service;
use axum::serve::ListenerExt;
use base64::Engine;
use percent_encoding::percent_decode;
use std::str::FromStr;
use tokio::process::Command;

use std::sync::LazyLock;

#[derive(Parser, Debug)]
#[clap(
    name = "static-server",
    about = "A simple static file server written in Rust based on axum framework.",
    author,
    version,
    long_about = None
)]
struct Opt {
    /// set the log level
    #[clap(short = 'l', long = "log", default_value = "info")]
    log_level: String,

    /// set the root directory
    #[clap(short = 'r', long = "root", default_value = ".")]
    root_dir: String,

    // enable video thumbnail
    #[clap(short = 't', long = "thumb", default_value = "false")]
    thumbnail: bool,

    /// set the listen addr
    #[clap(short = 'a', long = "addr", default_value = "127.0.0.1")]
    addr: String,

    /// set the listen port
    #[clap(short = 'p', long = "port", default_value = "3000")]
    port: u16,
}

#[derive(Clone, Default)]
struct StaticServerConfig {
    pub(crate) root_dir: String,
    pub(crate) thumbnail: bool,
}

// Add static variable for favicon using Lazy
static PIXEL_FAVICON: LazyLock<Vec<u8>> = LazyLock::new(|| {
    // one pixel favicon generated from https://png-pixel.com/
    let one_pixel_favicon = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mPk+89QDwADvgGOSHzRgAAAAABJRU5ErkJggg==";
    base64::prelude::BASE64_STANDARD.decode(one_pixel_favicon).unwrap()
});

// Add static variable for video thumbnail using Lazy
static VIDEO_THUMBNAIL: LazyLock<Vec<u8>> = LazyLock::new(|| include_bytes!("../templates/assets/play.png").to_vec());

static IS_VAAPI_SUPPORTED: LazyLock<bool> = LazyLock::new(check_vaapi_support);

fn check_vaapi_support() -> bool {
    tracing::info!("Checking available ffmpeg hwaccels...");
    match std::process::Command::new("ffmpeg")
        .arg("-hwaccels")
        .output() // Execute and capture output
    {
        Ok(output) => {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let supported = stdout.lines().any(|line| line.trim() == "vaapi");
                tracing::info!("VA-API available in ffmpeg -hwaccels: {}", supported);
                supported
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::error!("ffmpeg -hwaccels failed with status: {}\nStderr: {}", output.status, stderr);
                false
            }
        }
        Err(e) => {
            tracing::error!("Failed to execute ffmpeg -hwaccels: {}", e);
            false // ffmpeg command failed (e.g., not found), assume no support
        }
    }
}

#[tokio::main]
async fn main() {
    let opt = Opt::parse();

    // Set the RUST_LOG, if it hasn't been explicitly defined
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", format!("static_server={},tower_http={}", opt.log_level, opt.log_level))
    }
    tracing_subscriber::fmt::init();

    tracing::debug!("opt={:#?}", opt);

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
        .route("/frame", get(video_frame_thumbnail))
        .nest_service("/assets", get_service(ServeDir::new("./templates/assets")))
        .fallback(index_or_content)
        .layer(TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
            let ConnectInfo(addr) = request.extensions().get::<ConnectInfo<SocketAddr>>().unwrap();
            tracing::debug_span!("req", addr = %addr, path=%request.uri().path(), query=%request.uri().query().map(|q| format!("?{}", q)).unwrap_or_default())
        }))
        .with_state(StaticServerConfig {
            root_dir,
            thumbnail: opt.thumbnail,
        });

    let addr = std::net::IpAddr::from_str(opt.addr.as_str()).unwrap_or_else(|_| "127.0.0.1".parse().unwrap());

    let sock_addr = SocketAddr::from((addr, opt.port));

    tracing::info!("listening on http://{}", sock_addr);

    let listener = tokio::net::TcpListener::bind(sock_addr).await.unwrap().tap_io(|tcp_stream| {
        if let Err(err) = tcp_stream.set_nodelay(true) {
            tracing::error!("failed to set TCP_NODELAY on incoming connection: {err:#}");
        }
    });

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
}

// see https://kubernetes.io/docs/reference/using-api/health-checks/
async fn health_check() -> impl IntoResponse {
    "ok"
}

async fn favicon() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], PIXEL_FAVICON.clone())
}

async fn video_frame_thumbnail(State(cfg): State<StaticServerConfig>, Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    if !cfg.thumbnail {
        tracing::debug!("thumbnail generation disabled, return default");
        return ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], VIDEO_THUMBNAIL.clone());
    }

    let empty_val = &"".to_string();
    let file_path = params.get("file").unwrap_or(empty_val);

    let t = params.get("t").unwrap_or(&"30.0".to_string()).parse::<f64>().unwrap_or(30.0);
    let width = params.get("w").unwrap_or(&"1280".to_string()).parse::<u32>().unwrap_or(1280);

    let file_path = format!("{}/{}", cfg.root_dir, &file_path);
    tracing::info!("video_frame_thumbnail file_path={} width={}", &file_path, width);

    // https://www.ffmpeg.org/ffmpeg.html
    // ffmpeg -i ./Big_Buck_Bunny_360_10s_30MB.mp4 -ss 00:00:30.000 -vframes 1 -
    let child = Command::new("ffmpeg")
        // Exit after ffmpeg has been running for duration seconds in CPU user time.
        .arg("-timelimit")
        .arg("24")
        .arg("-loglevel")
        .arg("error")
        // Don't expect any audio in the stream
        .arg("-an")
        // Get the data from stdin
        .arg("-noautorotate")
        // Seek to the specified timestamp *before* opening the input file for faster seeking
        .arg("-ss")
        .arg(format!("00:00:{}.0", t))
        .arg("-i")
        .arg(&file_path)
        .arg("-vf")
        .arg(format!("scale={}:-1", width))
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("image2")
        // .arg("-o")
        .arg("-")
        // stdin, stderr, and stdout are piped
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    match child.wait_with_output().await {
        Ok(out) => {
            tracing::info!("video_frame_thumbnail ok file_path={}", &file_path);
            if out.status.success() {
                let stdout = out.stdout;
                tracing::info!("video_frame_thumbnail success file_path={}", &file_path);
                if !stdout.is_empty() {
                    ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], stdout)
                } else {
                    ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], VIDEO_THUMBNAIL.clone())
                }
            } else {
                let stdout = out.stdout;
                let stderr = out.stderr;
                tracing::error!(
                    "video_frame_thumbnail failed, code={:?} stderr={} stdout={}",
                    out.status.code(),
                    String::from_utf8_lossy(&stderr),
                    String::from_utf8_lossy(&stdout)
                );
                ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], VIDEO_THUMBNAIL.clone())
            }
        }
        Err(e) => {
            tracing::error!("video_frame_thumbnail error, file_path={} err={}", &file_path, e);
            ([(header::CONTENT_TYPE, HeaderValue::from_static("image/png"))], VIDEO_THUMBNAIL.clone())
        }
    }
}

// Request<Body> used an extractors cannot be combined with other unless Request<Body> is the very last extractor.
// see https://docs.rs/axum/latest/axum/extract/index.html#applying-multiple-extractors
// see https://github.com/tokio-rs/axum/discussions/583#discussioncomment-1739582
#[debug_handler]
async fn index_or_content(State(cfg): State<StaticServerConfig>, req: Request<Body>) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    match ServeDir::new(&cfg.root_dir).oneshot(req).await {
        Ok(res) => {
            let status = res.status();
            match status {
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
                _ => Ok(res.map(axum::body::Body::new)),
            }
        }
        Err(err) => Err(ErrorTemplate {
            err: InternalError(format!("Unhandled error: {}", err)),
            cur_path: path.to_string(),
            message: format!("Unhandled error: {}", err),
        }),
    }
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
            mime_type: mime_guess::from_path(child.path()).first_or_octet_stream().type_().to_string(),
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
        Err(askama::Error::Fmt)
    }
}

#[derive(Template)]
#[template(path = "index.html", print = "code")]
struct DirListTemplate {
    lister: DirLister,
    cur_path: String,
}

#[derive(Template)]
#[template(path = "error.html", print = "code")]
struct ErrorTemplate {
    err: ResponseError,
    cur_path: String,
    message: String,
}

const FAIL_REASON_HEADER_NAME: &str = "static-server-fail-reason";

impl IntoResponse for ErrorTemplate {
    fn into_response(self) -> Response<Body> {
        let t = self;
        match t.render() {
            Ok(html) => {
                let mut resp = Html(html).into_response();
                match t.err {
                    ResponseError::FileNotFound(reason) => {
                        *resp.status_mut() = StatusCode::NOT_FOUND;
                        resp.headers_mut().insert(FAIL_REASON_HEADER_NAME, reason.parse().unwrap());
                    }
                    ResponseError::BadRequest(reason) => {
                        *resp.status_mut() = StatusCode::BAD_REQUEST;
                        resp.headers_mut().insert(FAIL_REASON_HEADER_NAME, reason.parse().unwrap());
                    }
                    ResponseError::InternalError(reason) => {
                        *resp.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                        resp.headers_mut().insert(FAIL_REASON_HEADER_NAME, reason.parse().unwrap());
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
    mime_type: String,
    // path: String,
    path_uri: String,
    is_file: bool,
    last_modified: i64,
}

impl IntoResponse for DirListTemplate {
    fn into_response(self) -> Response<Body> {
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
