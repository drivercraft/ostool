//! HTTP throughput service integration tests.

use std::{io, net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::ConnectInfo,
    http::{Method, Request, StatusCode},
    response::Response,
};
use futures_util::{StreamExt, stream};
use ostool_server::{config::NetworkTestConfig, network_test::build_router};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinHandle;
use tower::ServiceExt;

fn request(method: Method, uri: impl AsRef<str>, body: Body) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(uri.as_ref())
        .body(body)
        .unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 12345))));
    request
}

async fn send(app: &Router, method: Method, uri: impl AsRef<str>, body: Body) -> Response {
    app.clone()
        .oneshot(request(method, uri, body))
        .await
        .unwrap()
}

async fn json(response: Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn create(app: &Router) -> String {
    let response = send(app, Method::POST, "/v1/tests", Body::empty()).await;
    assert_eq!(response.status(), StatusCode::CREATED);
    json(response).await["test_id"].as_str().unwrap().to_owned()
}

async fn snapshot(app: &Router, id: &str) -> Value {
    let response = send(app, Method::GET, format!("/v1/tests/{id}"), Body::empty()).await;
    assert_eq!(response.status(), StatusCode::OK);
    json(response).await
}

async fn wait_for_status(app: &Router, id: &str, direction: &str, expected: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if snapshot(app, id).await[direction]["status"] == expected {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{direction} for test {id} did not become {expected}"));
}

fn pending_upload(app: Router, id: String) -> JoinHandle<Response> {
    tokio::spawn(async move {
        let body = Body::from_stream(stream::pending::<Result<Bytes, io::Error>>());
        send(&app, Method::PUT, format!("/v1/tests/{id}/upload"), body).await
    })
}

#[tokio::test]
async fn concurrent_tests_reject_the_65th_and_release_a_slot_after_cancel() {
    let app = build_router(NetworkTestConfig::default());
    let mut uploads = Vec::new();
    for _ in 0..64 {
        let id = create(&app).await;
        let upload = pending_upload(app.clone(), id.clone());
        wait_for_status(&app, &id, "upload", "running").await;
        uploads.push((id, upload));
    }

    let waiting_id = create(&app).await;
    let response = send(
        &app,
        Method::PUT,
        format!("/v1/tests/{waiting_id}/upload"),
        Body::empty(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        snapshot(&app, &waiting_id).await["upload"]["status"],
        "not_started"
    );

    let (canceled_id, canceled_upload) = uploads.pop().unwrap();
    canceled_upload.abort();
    assert!(canceled_upload.await.unwrap_err().is_cancelled());
    wait_for_status(&app, &canceled_id, "upload", "canceled").await;
    assert_eq!(
        snapshot(&app, &canceled_id).await["upload"]["error"],
        "transfer canceled before completion"
    );
    let response = send(
        &app,
        Method::PUT,
        format!("/v1/tests/{waiting_id}/upload"),
        Body::empty(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await["status"], "completed");

    for (_, upload) in uploads {
        upload.abort();
        assert!(upload.await.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn both_directions_run_on_one_test_and_each_starts_only_once() {
    let app = build_router(NetworkTestConfig {
        max_active_tests: 1,
        ..NetworkTestConfig::default()
    });
    let id = create(&app).await;
    let upload = pending_upload(app.clone(), id.clone());
    wait_for_status(&app, &id, "upload", "running").await;

    let download = send(
        &app,
        Method::GET,
        format!("/v1/tests/{id}/download?bytes=131073"),
        Body::empty(),
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()["content-length"], "131073");
    assert_eq!(
        to_bytes(download.into_body(), 131073).await.unwrap().len(),
        131073
    );
    let state = snapshot(&app, &id).await;
    assert_eq!(state["upload"]["status"], "running");
    assert_eq!(state["download"]["status"], "completed");
    assert_eq!(state["download"]["bytes"], 131073);
    assert!(state["download"]["bits_per_second"].as_f64().unwrap() > 0.0);

    let duplicate = send(
        &app,
        Method::GET,
        format!("/v1/tests/{id}/download?bytes=1"),
        Body::empty(),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let duplicate = send(
        &app,
        Method::PUT,
        format!("/v1/tests/{id}/upload"),
        Body::empty(),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    upload.abort();
    assert!(upload.await.unwrap_err().is_cancelled());
    wait_for_status(&app, &id, "upload", "canceled").await;
}

#[tokio::test]
async fn exact_length_tcp_download_is_completed_after_hyper_closes_the_body() {
    let app = build_router(NetworkTestConfig::default());
    let id = create(&app).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            server_app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    });
    let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
    client
        .write_all(
            format!(
                "GET /v1/tests/{id}/download?bytes=65536 HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), client.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
    let body_start = response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(response.len() - body_start, 65536);
    wait_for_status(&app, &id, "download", "completed").await;
    assert_eq!(snapshot(&app, &id).await["download"]["bytes"], 65536);
    server.abort();
}

#[tokio::test]
async fn dropping_download_body_marks_canceled_and_releases_capacity() {
    let app = build_router(NetworkTestConfig {
        max_active_tests: 1,
        ..NetworkTestConfig::default()
    });
    let id = create(&app).await;
    let download = send(
        &app,
        Method::GET,
        format!("/v1/tests/{id}/download?bytes=1073741824"),
        Body::empty(),
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    let mut stream = download.into_body().into_data_stream();
    assert!(!stream.next().await.unwrap().unwrap().is_empty());
    drop(stream);
    wait_for_status(&app, &id, "download", "canceled").await;
    assert_eq!(
        snapshot(&app, &id).await["download"]["error"],
        "transfer canceled before completion"
    );

    let next_id = create(&app).await;
    let response = send(
        &app,
        Method::PUT,
        format!("/v1/tests/{next_id}/upload"),
        Body::empty(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn large_upload_counts_streamed_chunks_without_buffering_the_body() {
    const CHUNK_SIZE: usize = 64 * 1024;
    const TOTAL_BYTES: u64 = 65 * 1024 * 1024 + 17;
    let app = build_router(NetworkTestConfig::default());
    let id = create(&app).await;
    let chunk = Bytes::from(vec![0x5a; CHUNK_SIZE]);
    let chunks = stream::unfold(TOTAL_BYTES, move |remaining| {
        let chunk = chunk.clone();
        async move {
            if remaining == 0 {
                return None;
            }
            let size = remaining.min(CHUNK_SIZE as u64) as usize;
            Some((
                Ok::<_, io::Error>(chunk.slice(..size)),
                remaining - size as u64,
            ))
        }
    });
    let response = send(
        &app,
        Method::PUT,
        format!("/v1/tests/{id}/upload"),
        Body::from_stream(chunks),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let result = json(response).await;
    assert_eq!(result["status"], "completed");
    assert_eq!(result["bytes"], TOTAL_BYTES);
    assert_eq!(snapshot(&app, &id).await["upload"]["bytes"], TOTAL_BYTES);
}

#[tokio::test]
async fn invalid_requests_and_body_errors_keep_queryable_results() {
    let app = build_router(NetworkTestConfig::default());
    assert_eq!(
        send(&app, Method::GET, "/healthz", Body::empty())
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        send(
            &app,
            Method::GET,
            "/v1/tests/00000000-0000-0000-0000-000000000000",
            Body::empty(),
        )
        .await
        .status(),
        StatusCode::NOT_FOUND
    );
    let id = create(&app).await;
    for query in [
        "",
        "?bytes=0",
        "?duration_secs=0",
        "?bytes=1&duration_secs=1",
        "?duration_secs=3601",
    ] {
        let response = send(
            &app,
            Method::GET,
            format!("/v1/tests/{id}/download{query}"),
            Body::empty(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "query: {query}");
    }
    assert_eq!(
        snapshot(&app, &id).await["download"]["status"],
        "not_started"
    );

    let body = Body::from_stream(stream::once(async {
        Err::<Bytes, _>(io::Error::other("injected body failure"))
    }));
    let response = send(&app, Method::PUT, format!("/v1/tests/{id}/upload"), body).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let state = snapshot(&app, &id).await;
    assert_eq!(state["upload"]["status"], "failed");
    assert!(
        state["upload"]["error"]
            .as_str()
            .unwrap()
            .contains("injected body failure")
    );
}

#[tokio::test]
async fn upload_timeout_releases_capacity_and_duration_download_completes() {
    let app = build_router(NetworkTestConfig {
        max_active_tests: 1,
        max_duration_secs: 1,
        ..NetworkTestConfig::default()
    });
    let id = create(&app).await;
    let upload = pending_upload(app.clone(), id.clone());
    wait_for_status(&app, &id, "upload", "running").await;
    let response = tokio::time::timeout(Duration::from_secs(3), upload)
        .await
        .expect("upload should time out")
        .unwrap();
    assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
    assert_eq!(snapshot(&app, &id).await["upload"]["status"], "timed_out");

    let next_id = create(&app).await;
    let download = send(
        &app,
        Method::GET,
        format!("/v1/tests/{next_id}/download?duration_secs=1"),
        Body::empty(),
    )
    .await;
    assert_eq!(download.status(), StatusCode::OK);
    let received = tokio::time::timeout(Duration::from_secs(3), async {
        let mut stream = download.into_body().into_data_stream();
        let mut received = 0_u64;
        while let Some(chunk) = stream.next().await {
            received += chunk.unwrap().len() as u64;
        }
        received
    })
    .await
    .expect("duration download should finish");
    assert!(received > 0);
    let state = snapshot(&app, &next_id).await;
    assert_eq!(state["download"]["status"], "completed");
    assert_eq!(state["download"]["bytes"], received);

    let stalled_id = create(&app).await;
    let stalled = send(
        &app,
        Method::GET,
        format!("/v1/tests/{stalled_id}/download?bytes=1073741824"),
        Body::empty(),
    )
    .await;
    assert_eq!(stalled.status(), StatusCode::OK);
    wait_for_status(&app, &stalled_id, "download", "timed_out").await;
    assert_eq!(
        snapshot(&app, &stalled_id).await["download"]["error"],
        "maximum test duration reached"
    );
    let after_timeout_id = create(&app).await;
    let response = send(
        &app,
        Method::PUT,
        format!("/v1/tests/{after_timeout_id}/upload"),
        Body::empty(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(stalled);
}
