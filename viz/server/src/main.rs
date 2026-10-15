mod runner;
mod sim;
mod testgen;

use std::{env, path::PathBuf, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tower_http::services::ServeDir;

use crate::sim::{Expect, Sim, Who};

type Shared = Arc<Mutex<Sim>>;
type ApiResult<T> = Result<Json<T>, ApiError>;

#[derive(Debug)]
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.0, Json(json!({"error": self.1}))).into_response()
    }
}
fn bad(message: &str) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, message.into())
}
fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("内部エラー: {e}"),
    )
}

#[derive(Deserialize)]
struct SendRequest {
    from: Who,
    text: String,
    deliver: bool,
}
#[derive(Deserialize)]
struct IdRequest {
    message_id: u32,
}
#[derive(Deserialize)]
struct ExpectRequest {
    expect: Expect,
}
#[derive(Deserialize)]
struct NameRequest {
    name: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let sim = Arc::new(Mutex::new(Sim::new()?));
    let app = app_router(sim);
    let addr = env::var("VIZ_ADDR").unwrap_or_else(|_| {
        if std::path::Path::new("/.dockerenv").exists() {
            "0.0.0.0:8787".into()
        } else {
            "127.0.0.1:8787".into()
        }
    });
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    eprintln!("libsignal viz server: http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn app_router(sim: Shared) -> Router {
    let api = Router::new()
        .route("/state", get(state))
        .route("/reset", post(reset))
        .route("/send", post(send))
        .route("/deliver", post(deliver))
        .route("/tamper", post(tamper))
        .route("/replay", post(replay))
        .route("/script/{i}/expect", put(set_expect))
        .route("/test/generate", post(generate))
        .route("/test/run", post(run))
        .route("/tests", get(tests))
        .fallback(|| async {
            ApiError(StatusCode::NOT_FOUND, "APIが見つかりません".into())
        })
        .method_not_allowed_fallback(|| async {
            ApiError(
                StatusCode::METHOD_NOT_ALLOWED,
                "このHTTPメソッドは使用できません".into(),
            )
        });
    Router::new()
        .nest("/api", api)
        .fallback_service(ServeDir::new(repo_root().join("viz/web")))
        .with_state(sim)
}

async fn state(State(sim): State<Shared>) -> ApiResult<sim::State> {
    Ok(Json(sim.lock().await.state()))
}
async fn reset(State(sim): State<Shared>) -> ApiResult<sim::State> {
    let mut guard = sim.lock().await;
    *guard = Sim::new().map_err(internal)?;
    Ok(Json(guard.state()))
}
async fn send(
    State(sim): State<Shared>,
    body: Result<Json<SendRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<sim::State> {
    let Json(r) = body.map_err(|_| bad("リクエストのJSONが正しくありません"))?;
    if r.text.chars().count() == 0 || r.text.chars().count() > 200 {
        return Err(bad("本文は1〜200文字で入力してください"));
    }
    let mut s = sim.lock().await;
    s.send(r.from, r.text, r.deliver).map_err(internal)?;
    Ok(Json(s.state()))
}
async fn deliver(
    State(sim): State<Shared>,
    body: Result<Json<IdRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<sim::State> {
    alter(sim, body, |s, r| s.deliver(r.message_id)).await
}
async fn tamper(
    State(sim): State<Shared>,
    body: Result<Json<IdRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<sim::State> {
    alter(sim, body, |s, r| s.tamper(r.message_id)).await
}
async fn replay(
    State(sim): State<Shared>,
    body: Result<Json<IdRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<sim::State> {
    alter(sim, body, |s, r| s.replay(r.message_id)).await
}
async fn alter<F>(
    sim: Shared,
    body: Result<Json<IdRequest>, axum::extract::rejection::JsonRejection>,
    f: F,
) -> ApiResult<sim::State>
where
    F: FnOnce(&mut Sim, IdRequest) -> anyhow::Result<()>,
{
    let Json(r) = body.map_err(|_| bad("リクエストのJSONが正しくありません"))?;
    let mut s = sim.lock().await;
    f(&mut s, r).map_err(|e| ApiError(StatusCode::CONFLICT, e.to_string()))?;
    Ok(Json(s.state()))
}
async fn set_expect(
    State(sim): State<Shared>,
    path: Result<Path<usize>, axum::extract::rejection::PathRejection>,
    body: Result<Json<ExpectRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<sim::State> {
    let Path(i) = path.map_err(|_| bad("操作番号が正しくありません"))?;
    let Json(r) = body.map_err(|_| bad("リクエストのJSONが正しくありません"))?;
    let mut s = sim.lock().await;
    s.set_expect(i, r.expect)
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(s.state()))
}
async fn generate(
    State(sim): State<Shared>,
    body: Result<Json<NameRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Value> {
    let Json(r) = body.map_err(|_| bad("リクエストのJSONが正しくありません"))?;
    if !valid_name(&r.name) {
        return Err(bad(
            "テスト名は英小文字で始まる40文字以内の英小文字・数字・_にしてください",
        ));
    }
    let s = sim.lock().await;
    if s.script.is_empty() {
        return Err(bad("操作を1つ以上行ってから生成してください"));
    }
    let code = testgen::generate(&r.name, &s.script).map_err(internal)?;
    drop(s);
    let path = repo_root()
        .join("rust/protocol/tests")
        .join(format!("viz_{}.rs", r.name));
    // Never overwrite a hand-written test such as tests/viz_trace.rs.
    if let Ok(existing) = tokio::fs::read_to_string(&path).await
        && !existing.starts_with("// generated by the viz tool")
    {
        return Err(ApiError(
            StatusCode::CONFLICT,
            format!(
                "viz_{}.rs は手書きのテストなので上書きできません。別の名前にしてください",
                r.name
            ),
        ));
    }
    tokio::fs::write(&path, &code).await.map_err(internal)?;
    runner::format_generated(&path).await.map_err(internal)?;
    let code = tokio::fs::read_to_string(&path).await.map_err(internal)?;
    Ok(Json(json!({"name":r.name,"path":path,"code":code})))
}
async fn run(
    body: Result<Json<NameRequest>, axum::extract::rejection::JsonRejection>,
) -> ApiResult<Value> {
    let Json(r) = body.map_err(|_| bad("リクエストのJSONが正しくありません"))?;
    if !valid_name(&r.name) {
        return Err(bad("テスト名が正しくありません"));
    }
    let result = runner::run(&r.name).await.map_err(|e| {
        if e.is::<runner::Busy>() {
            ApiError(StatusCode::CONFLICT, e.to_string())
        } else {
            internal(e)
        }
    })?;
    Ok(Json(serde_json::to_value(result).map_err(internal)?))
}
async fn tests() -> ApiResult<Value> {
    let mut entries = tokio::fs::read_dir(repo_root().join("rust/protocol/tests"))
        .await
        .map_err(internal)?;
    let mut tests = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(internal)? {
        let n = entry.file_name().to_string_lossy().to_string();
        if entry.file_type().await.map_err(internal)?.is_file()
            && n.starts_with("viz_")
            && n.ends_with(".rs")
        {
            let name = n
                .strip_prefix("viz_")
                .and_then(|n| n.strip_suffix(".rs"))
                .expect("checked filename");
            tests.push((name.to_owned(), entry.path()));
        }
    }
    tests.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Json(
        json!({"tests":tests.into_iter().map(|(name,path)| json!({"name":name,"path":path})).collect::<Vec<_>>()}),
    ))
}
fn valid_name(n: &str) -> bool {
    let b = n.as_bytes();
    !b.is_empty()
        && b.len() <= 40
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}
fn repo_root() -> PathBuf {
    env::var_os("VIZ_REPO_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .canonicalize()
                .expect("repo root")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn api_errors_are_japanese_json() -> anyhow::Result<()> {
        let app = app_router(Arc::new(Mutex::new(Sim::new()?)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        for (method, path, body, expected) in [
            ("GET", "/api/unknown", "", 404),
            ("PATCH", "/api/state", "", 405),
            ("PUT", "/api/script/nope/expect", r#"{"expect":"ok"}"#, 400),
            ("POST", "/api/send", "broken", 400),
            (
                "POST",
                "/api/send",
                r#"{"from":"alice","text":"","deliver":true}"#,
                400,
            ),
            ("POST", "/api/test/generate", r#"{"name":"demo"}"#, 400),
            ("POST", "/api/test/generate", r#"{"name":"../escape"}"#, 400),
            ("POST", "/api/deliver", r#"{"message_id":0}"#, 409),
        ] {
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            stream.write_all(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
            let mut response = String::new();
            stream.read_to_string(&mut response).await?;
            let (headers, body) = response.split_once("\r\n\r\n").expect("HTTP response");
            assert!(
                headers.starts_with(&format!("HTTP/1.1 {expected}")),
                "{path}: {headers}"
            );
            assert!(headers.contains("application/json"), "{path}: {headers}");
            let value: Value = serde_json::from_str(body)?;
            assert!(
                !value["error"].as_str().expect("error string").is_ascii(),
                "{path}: {body}"
            );
        }
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn generate_refuses_to_overwrite_hand_written_test() -> anyhow::Result<()> {
        let hand_written = repo_root().join("rust/protocol/tests/viz_trace.rs");
        let before = tokio::fs::read_to_string(&hand_written).await?;
        let app = app_router(Arc::new(Mutex::new(Sim::new()?)));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move { axum::serve(listener, app).await });
        let mut statuses = Vec::new();
        for (path, body) in [
            (
                "/api/send",
                r#"{"from":"alice","text":"hi","deliver":true}"#,
            ),
            ("/api/test/generate", r#"{"name":"trace"}"#),
        ] {
            let mut stream = tokio::net::TcpStream::connect(address).await?;
            stream.write_all(format!("POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
            let mut response = String::new();
            stream.read_to_string(&mut response).await?;
            statuses.push(response.lines().next().unwrap_or_default().to_owned());
        }
        server.abort();
        assert!(statuses[0].starts_with("HTTP/1.1 200"), "{statuses:?}");
        assert!(statuses[1].starts_with("HTTP/1.1 409"), "{statuses:?}");
        assert_eq!(tokio::fs::read_to_string(&hand_written).await?, before);
        Ok(())
    }
}
