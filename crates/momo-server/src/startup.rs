use super::*;

pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "momo_server=info,tower_http=info".into()),
        )
        .init();

    let bind = env::var("MOMO_SERVER_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    let addr: SocketAddr = bind.parse()?;
    ensure_bind_allowed(addr, env_flag(ALLOW_REMOTE_ENV))?;
    let data_dir = env::var("MOMO_DATA_DIR").unwrap_or_else(|_| DEFAULT_DATA_DIR.to_owned());
    let runtime = Arc::new(MomoRuntime::initialize(&data_dir).await?);
    let gateway_origin = env::var("MOMO_MODEL_GATEWAY_ORIGIN")
        .unwrap_or_else(|_| "http://127.0.0.1:8788/v1".to_owned());
    let gateway_api_key = env::var("MOMO_MODEL_GATEWAY_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let gateway_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let momo_api = Arc::new(MomoApiService::new(
        Arc::clone(&runtime),
        gateway_origin.clone(),
        gateway_api_key.clone(),
        gateway_client.clone(),
    ));
    let state = AppState {
        runtime,
        momo_api,
        response_concurrency: Arc::new(Semaphore::new(env_usize(
            "MOMO_RESPONSE_MAX_CONCURRENCY",
            8,
            1,
            256,
        )?)),
        response_timeout: std::time::Duration::from_secs(env_usize(
            "MOMO_RESPONSE_TIMEOUT_SECONDS",
            120,
            1,
            3_600,
        )? as u64),
        metrics: Arc::new(Mutex::new(HashMap::new())),
    };

    let momo_api = Arc::clone(&state.momo_api);
    let app = build_app(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("MOMO server listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    momo_api.wait_for_maintenance().await;
    Ok(())
}

pub(crate) fn env_flag(name: &str) -> bool {
    env::var(name).is_ok_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"))
}

pub(crate) fn env_usize(
    name: &str,
    default: usize,
    min: usize,
    max: usize,
) -> std::io::Result<usize> {
    let value = match env::var(name) {
        Ok(value) => value.parse::<usize>().map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{name} must be an integer between {min} and {max}"),
            )
        })?,
        Err(env::VarError::NotPresent) => default,
        Err(error) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("failed to read {name}: {error}"),
            ));
        }
    };
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{name} must be between {min} and {max}"),
        ))
    }
}

pub(crate) fn ensure_bind_allowed(addr: SocketAddr, allow_remote: bool) -> std::io::Result<()> {
    if addr.ip().is_loopback() || allow_remote {
        return Ok(());
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        format!(
            "refusing non-loopback bind {addr}; set {ALLOW_REMOTE_ENV}=1 only behind a trusted authenticated proxy"
        ),
    ))
}
