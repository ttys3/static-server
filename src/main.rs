use axum_macros::debug_handler;
use std::collections::HashMap;
use std::net::SocketAddr;
use tower_http::trace::TraceLayer;

use crate::ResponseError::{BadRequest, FileNotFound, InternalError};
use rinja::Template;

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
use axum::extract::{ConnectInfo, Query};
use axum::routing::get_service;
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
static VIDEO_THUMBNAIL: LazyLock<Vec<u8>> = LazyLock::new(|| {
    // one pixel video thumbnail generated from base64 string
    let video_icon = "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";
    base64::prelude::BASE64_STANDARD.decode(video_icon).unwrap()
});

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

    let listener = tokio::net::TcpListener::bind(sock_addr).await.unwrap();

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .tcp_nodelay(true)
        .await
        .unwrap();
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
        .arg("-i")
        .arg(&file_path)
        .arg("-ss")
        .arg(format!("00:00:{}.0", t))
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
    return match ServeDir::new(&cfg.root_dir).oneshot(req).await {
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
    pub(crate) fn datetime(ts: &i64) -> ::rinja::Result<String> {
        if let Ok(format) = time::format_description::parse("[year]-[month]-[day] [hour]:[minute]:[second] UTC") {
            return Ok(time::OffsetDateTime::from_unix_timestamp(*ts).unwrap().format(&format).unwrap());
        }
        Err(rinja::Error::Fmt)
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
