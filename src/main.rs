use axum::{http::StatusCode, Router};
use std::{convert::Infallible, net::SocketAddr};
use tower_http::trace::TraceLayer;

// use axum::{extract::Path as extractPath};

use crate::ResponseType::{BadRequest, IndexPage};
use askama::Template;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request, HeaderMap};
use axum::{
    body::{Bytes, Full},
    handler::get,
    http::Response,
    response::{Html, IntoResponse},
};
use std::fs::File;
use std::io::{Read, Error};
use std::path::{Path, PathBuf};
use tower_http::services::ServeDir;
use tower::util::ServiceExt;

use std::{io};
use tokio::fs::{self, DirEntry};
use std::ffi::OsStr;
use tower_http::services;

#[tokio::main]
async fn main() {
    // Set the RUST_LOG, if it hasn't been explicitly defined
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "static_server=debug,tower_http=debug")
    }
    tracing_subscriber::fmt::init();

    let root_dir = ".".to_string();

    let app = Router::new()
        .nest("/", get(|req: Request<Body>| async move {
            let path = req.uri().path().to_string();
            return match ServeDir::new(&root_dir).oneshot(req).await {
                Ok(res) => {
                    match res.status() {
                        StatusCode::NOT_FOUND => {
                            let path = path.trim_start_matches('/');

                            let mut full_path = PathBuf::new();
                            full_path.push(&root_dir);
                            for seg in path.split('/') {
                                if seg.starts_with("..") || seg.contains('\\') {
                                    let body = HtmlTemplate(DirListTemplate {
                                        resp: BadRequest("invalid path".to_string()),
                                        cur_path: path.to_string(),
                                    }).into_response();
                                    return axum::body::box_body(body)
                                }
                                full_path.push(seg);
                            }

                            let cur_path = Path::new(&full_path);

                            match cur_path.is_dir() {
                                true => {
                                    let rs = visit_dir_one_level(&full_path).await;
                                    match rs {
                                        Ok(files) => {
                                            let body = HtmlTemplate(DirListTemplate {
                                                resp: IndexPage(DirLister { files }),
                                                cur_path: path.to_string(),
                                            }).into_response();
                                            axum::body::box_body(body)
                                        }
                                        Err(e) => {
                                            let body = HtmlTemplate(DirListTemplate {
                                                resp: BadRequest(e.to_string()),
                                                cur_path: path.to_string(),
                                            }).into_response();
                                            axum::body::box_body(body)
                                        }
                                    }
                                },
                                false => {
                                    let body = HtmlTemplate(DirListTemplate {
                                        resp: BadRequest("file not found".to_string()),
                                        cur_path: path.to_string(),
                                    }).into_response();
                                    axum::body::box_body(body)
                                }
                            }
                        }
                        _ => {
                            axum::body::box_body(res)
                        }
                    }

                },
                Err(err) => {
                    let body = HtmlTemplate(DirListTemplate {
                        resp: BadRequest(format!("Unhandled error: {}", err)),
                        cur_path: path.to_string(),
                    }).into_response();
                    axum::body::box_body(body)
                },
            };
        }))
        .route("/favicon.ico", get(favicon))
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    tracing::info!("listening on http://{}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

// io::Result<Vec<DirEntry>>
async fn visit_dir_one_level(path: &PathBuf) -> io::Result<Vec<FileInfo>> {
    let mut dir = fs::read_dir(path).await?;
    // let mut files = Vec::new();
    let mut files: Vec<FileInfo> = Vec::new();

    while let Some(child) = dir.next_entry().await? {
        // files.push(child)
        files.push(FileInfo {
            name: child
                .file_name()
                .to_string_lossy()
                .to_string(),
            ext: Path::new(child.file_name().to_str().unwrap()).extension()
                .and_then(OsStr::to_str).unwrap_or_default().to_string(),
            path: child.path().to_string_lossy().to_string(),
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
    res.headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
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
    is_file: bool,
    last_modified: u64,
}

struct HtmlTemplate(DirListTemplate);

impl IntoResponse for HtmlTemplate {
    type Body = Full<Bytes>;
    type BodyError = Infallible;

    fn into_response(self) -> Response<Self::Body> {
        let t = self.0;
        match t.resp {
            ResponseType::BadRequest(msg) => Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body(Full::from(msg))
                .unwrap(),
            ResponseType::IndexPage(_) => match t.render() {
                Ok(html) => Html(html).into_response(),
                Err(err) => {
                    tracing::error!("template render failed, err={}", err);
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Full::from(format!(
                            "Failed to render template. Error: {}",
                            err
                        )))
                        .unwrap()
                }
            },
        }
    }
}
