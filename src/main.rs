use axum::{http::StatusCode, Router};
use std::{convert::Infallible, fs, net::SocketAddr};
use tower_http::trace::TraceLayer;

// use axum::{extract::Path as extractPath};

use crate::ResponseType::{BadRequest, DownloadFile, IndexPage};
use askama::Template;
use axum::body::Body;
use axum::http::{header, HeaderValue, Request};
use axum::{
    body::{Bytes, Full},
    handler::get,
    http::Response,
    response::{Html, IntoResponse},
};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

#[tokio::main]
async fn main() {
    // Set the RUST_LOG, if it hasn't been explicitly defined
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "static_server=debug,tower_http=debug")
    }
    tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/.*", get(serve_files))
        .route("/favicon.ico", get(favicon))
        .layer(TraceLayer::new_for_http());

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("listening on http://{}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
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

// extractPath(the_req_path): extractPath<String>
async fn serve_files(req: Request<Body>) -> impl IntoResponse {
    let mut files: Vec<FileInfo> = Vec::new();

    // build and validate the path
    // let path = the_req_path;

    let path = req.uri().path();
    let path = path.trim_start_matches('/');
    let mut full_path = PathBuf::new();
    full_path.push(".");
    for seg in path.split('/') {
        if seg.starts_with("..") || seg.contains('\\') {
            return HtmlTemplate(DirListTemplate {
                resp: BadRequest("invalid path".to_string()),
                cur_path: path.to_string(),
            });
        }
        full_path.push(seg);
    }

    let cur_path = Path::new(&full_path);

    return match cur_path.is_dir() {
        true => {
            for entry in fs::read_dir(cur_path).unwrap() {
                if !entry.is_ok() {
                    continue;
                }
                let path = entry.unwrap().path();
                let item = Path::new(&path);

                let metadata = fs::metadata(&path).unwrap();
                let last_modified = metadata.modified().unwrap().elapsed().unwrap().as_secs();

                files.push(FileInfo {
                    name: item
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    ext: item
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    path: path.to_string_lossy().to_string(),
                    is_file: metadata.is_file(),
                    last_modified,
                });
            }

            let template = DirListTemplate {
                // resp: IndexPage(Tmpl{message: "ok".to_string(), is400: false, cur_path: path.to_string(), files }),
                resp: IndexPage(DirLister { files }),
                cur_path: path.to_string(),
            };
            HtmlTemplate(template)
        }
        false => {
            // ServeFile::new(cur_path)
            // ServeDir::new
            return HtmlTemplate(DirListTemplate {
                resp: DownloadFile(path.to_string()),
                cur_path: path.to_string(),
            });
        }
    };
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
    DownloadFile(String),
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
            ResponseType::DownloadFile(path) => {
                let guess = mime_guess::from_path(&path);
                let mime = guess
                    .first_raw()
                    .map(|mime| HeaderValue::from_static(mime))
                    .unwrap_or_else(|| {
                        HeaderValue::from_str(mime::APPLICATION_OCTET_STREAM.as_ref()).unwrap()
                    });

                match File::open(&path) {
                    Ok(mut f) => {
                        let mut buffer = Vec::new();
                        f.read_to_end(&mut buffer).unwrap();
                        let mut res = Response::new(Full::from(buffer));
                        res.headers_mut().insert(header::CONTENT_TYPE, mime);
                        res
                    }
                    Err(err) => {
                        tracing::error!("open file failed, err={}", err);
                        Response::builder()
                            .status(StatusCode::INTERNAL_SERVER_ERROR)
                            .body(Full::from(format!(
                                "Failed to open {} . Error: {}",
                                &path, err
                            )))
                            .unwrap()
                    }
                }
            }
        }
    }
}
