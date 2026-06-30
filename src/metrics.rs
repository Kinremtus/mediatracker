use axum::{
    extract::Request,
    extract::State,
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics::counter;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

#[derive(Clone)]
pub struct MetricsHandle(pub Arc<PrometheusHandle>);

impl MetricsHandle {
    pub fn render(&self) -> String {
        self.0.render()
    }
}

pub fn init_metrics() -> MetricsHandle {
    static HANDLE: OnceLock<MetricsHandle> = OnceLock::new();
    HANDLE
        .get_or_init(|| {
            let builder = PrometheusBuilder::new();
            let handle = builder
                .install_recorder()
                .expect("failed to install prometheus recorder");
            MetricsHandle(Arc::new(handle))
        })
        .clone()
}

pub async fn metrics_handler(state: State<MetricsHandle>) -> Response {
    let body = state.render();
    let mut response = Response::new(axum::body::Body::from(body));
    response.headers_mut().insert(
        "content-type",
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

pub async fn metrics_middleware(
    req: Request,
    next: Next,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let start = Instant::now();
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    let response = next.run(req).await;

    let duration = start.elapsed();
    let status = response.status().as_u16();

    counter!("http_requests_total", "method" => method.clone(), "path" => path, "status" => status.to_string())
        .increment(1);

    metrics::histogram!("http_request_duration_seconds", "method" => method, "status" => status.to_string())
        .record(duration.as_secs_f64());

    Ok(response)
}
