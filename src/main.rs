mod config;
mod http;
mod ocr;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config_path = PathBuf::from("configs/application.yaml");
    let cfg = config::AppConfig::load(&config_path);

    let engine_dir = PathBuf::from(&cfg.ocr.engine_dir);
    let env = ocr::init_ort_environment(cfg.ocr.ort_lib.as_deref(), &engine_dir)?;
    let engine = ocr::Engine::new(
        engine_dir,
        cfg.ocr.models.clone(),
        cfg.ocr.num_thread,
        cfg.ocr.rec_pool,
        env,
    )
    .context("初始化 OCR 引擎失败")?;
    let engine = Arc::new(engine);

    tracing::info!("正在预热 OCR 引擎（默认语言：{}）", engine.default_language());
    let warm = engine.clone();
    match tokio::task::spawn_blocking(move || warm.warmup()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("预热失败（服务仍会启动，首次识别时会重试）：{e:#}"),
        Err(e) => tracing::warn!("预热任务异常：{e}"),
    }

    let web_index = PathBuf::from(&cfg.server.web_dir).join("index.html");
    let app = http::router(engine, web_index);

    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.server.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("监听端口 {} 失败", cfg.server.port))?;
    tracing::info!("Umi-OCR HTTP API 已启动：http://127.0.0.1:{}", cfg.server.port);
    tracing::info!("按 Ctrl+C 停止服务");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("HTTP 服务异常退出")?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("收到退出信号，正在停止服务…");
}
