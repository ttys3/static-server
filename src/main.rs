use axum::{http::StatusCode, Router};
use std::{convert::Infallible, net::SocketAddr};
use tower_http::trace::TraceLayer;

// use axum::{extract::Path as extractPath};

use crate::ResponseType::{BadRequest, IndexPage};
use askama::Template;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request};
use axum::{
    body::{Bytes, Full},
    handler::get,
    http::Response,
    response::{Html, IntoResponse},
};

use std::path::{Path, PathBuf};
use tower::util::ServiceExt;
use tower_http::services::ServeDir;

use std::ffi::OsStr;
use std::io;
use tokio::fs;

use structopt::StructOpt;
// for IpAddr::from_str
use std::str::FromStr;
use axum::extract::ConnectInfo;

#[derive(Debug, StructOpt)]
#[structopt(name = "static-server", about = "A simple static file server written in Rust based on axum framework.")]
struct Opt {
    /// set the log level
    #[structopt(short = "l", long = "log", default_value = "debug")]
    log_level: String,

    /// set the root directory
    #[structopt(short = "r", long = "root", default_value = ".")]
    root_dir: String,

    /// set the listen addr
    #[structopt(short = "a", long = "addr", default_value = "127.0.0.1")]
    addr: String,

    /// set the listen port
    #[structopt(short = "p", long = "port", default_value = "3000")]
    port: u16,
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();
    tracing::debug!("opt={:?}", opt);

    // Set the RUST_LOG, if it hasn't been explicitly defined
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", format!("static_server={},tower_http={}", opt.log_level, opt.log_level))
    }
    tracing_subscriber::fmt::init();

    let root_dir = opt.root_dir;

    let app = Router::new()
        .nest(
            "/",
            get(|req: Request<Body>| async move {
                let path = req.uri().path().to_string();
                return match ServeDir::new(&root_dir).oneshot(req).await {
                    Ok(res) => match res.status() {
                        StatusCode::NOT_FOUND => {
                            let path = path.trim_start_matches('/');

                            let mut full_path = PathBuf::new();
                            full_path.push(&root_dir);
                            for seg in path.split('/') {
                                if seg.starts_with("..") || seg.contains('\\') {
                                    return HtmlTemplate(DirListTemplate {
                                        resp: BadRequest("invalid path".to_string()),
                                        cur_path: path.to_string(),
                                    })
                                    .into_response()
                                    .map(axum::body::box_body);
                                }
                                full_path.push(seg);
                            }

                            let cur_path = Path::new(&full_path);

                            match cur_path.is_dir() {
                                true => {
                                    let rs = visit_dir_one_level(&full_path, &root_dir).await;
                                    match rs {
                                        Ok(files) => HtmlTemplate(DirListTemplate {
                                            resp: IndexPage(DirLister { files }),
                                            cur_path: path.to_string(),
                                        })
                                        .into_response()
                                        .map(axum::body::box_body),
                                        Err(e) => HtmlTemplate(DirListTemplate {
                                            resp: BadRequest(e.to_string()),
                                            cur_path: path.to_string(),
                                        })
                                        .into_response()
                                        .map(axum::body::box_body),
                                    }
                                }
                                false => HtmlTemplate(DirListTemplate {
                                    resp: BadRequest("file not found".to_string()),
                                    cur_path: path.to_string(),
                                })
                                .into_response()
                                .map(axum::body::box_body),
                            }
                        }
                        _ => res.map(axum::body::box_body),
                    },
                    Err(err) => HtmlTemplate(DirListTemplate {
                        resp: BadRequest(format!("Unhandled error: {}", err)),
                        cur_path: path.to_string(),
                    })
                    .into_response()
                    .map(axum::body::box_body),
                };
            }),
        )
        .route("/favicon.ico", get(favicon))
        // .layer(TraceLayer::new_for_http())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                let ConnectInfo(addr) = request
                    .extensions()
                    .get::<ConnectInfo<SocketAddr>>()
                    .unwrap();
                let empty_val = &HeaderValue::from_static("");
                let user_agent = request.headers().get("User-Agent").unwrap_or(empty_val).to_str().unwrap_or("");
                tracing::debug_span!("client-addr", addr = %addr, user_agent=%user_agent)
            }),
        );

    let addr = std::net::IpAddr::from_str(opt.addr.as_str()).
        unwrap_or("127.0.0.1".parse().unwrap());

    let sock_addr = SocketAddr::from((addr, opt.port));

    tracing::info!("listening on http://{}", sock_addr);

    axum::Server::bind(&sock_addr)
        .serve(app.into_make_service_with_connect_info::<SocketAddr, _>())
        .await
        .unwrap();
}

// io::Result<Vec<DirEntry>>
async fn visit_dir_one_level(path: &Path, prefix: &str) -> io::Result<Vec<FileInfo>> {
    let mut dir = fs::read_dir(path).await?;
    // let mut files = Vec::new();
    let mut files: Vec<FileInfo> = Vec::new();

    while let Some(child) = dir.next_entry().await? {
        // files.push(child)

        let the_path = child.path().to_string_lossy().to_string();
        let mut the_uri_path = the_path.clone();
        if !prefix.is_empty() && !the_path.starts_with(prefix) {
            tracing::error!("visit_dir_one_level skip invalid path={}", the_path);
            continue;
        } else {
            the_uri_path = the_path.strip_prefix(prefix).unwrap().to_string();
        }
        files.push(FileInfo {
            name: child.file_name().to_string_lossy().to_string(),
            ext: Path::new(child.file_name().to_str().unwrap())
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_string(),
            path: the_path,
            path_uri: the_uri_path,
            is_file: child.file_type().await?.is_file(),
            last_modified: child.metadata().await?.modified().unwrap().elapsed().unwrap().as_secs(),
        });
    }

    Ok(files)
}

async fn favicon() -> impl IntoResponse {
    // one pixel favicon generated from https://png-pixel.com/
    let one_pixel_favicon = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mPk+89QDwADvgGOSHzRgAAAAABJRU5ErkJggg==";
    let pixel_favicon = base64::decode(one_pixel_favicon).unwrap();
    let mut res = Response::new(Full::from(pixel_favicon));
    res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    res
}

#[derive(Template)]
#[template(path = "index.html")]
struct DirListTemplate {
    resp: ResponseType,
    cur_path: String,
}

enum ResponseType {
    BadRequest(String),
    IndexPage(DirLister),
}

struct DirLister {
    files: Vec<FileInfo>,
}

struct FileInfo {
    name: String,
    ext: String,
    path: String,
    path_uri: String,
    is_file: bool,
    last_modified: u64,
}

struct HtmlTemplate(DirListTemplate);

impl IntoResponse for HtmlTemplate {
    type Body = Full<Bytes>;
    type BodyError = Infallible;

    fn into_response(self) -> Response<Self::Body> {
        let t = self.0;
        match t.render() {
            Ok(html) => {
                let mut resp = Html(html).into_response();
                if let ResponseType::BadRequest(_) = t.resp {
                    *resp.status_mut() = StatusCode::BAD_REQUEST;
                }
                resp
            }
            Err(err) => {
                tracing::error!("template render failed, err={}", err);
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Full::from(format!("Failed to render template. Error: {}", err)))
                    .unwrap()
            }
        }
    }
}
