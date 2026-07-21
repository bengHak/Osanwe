use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{bail, Context};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
    pub token: String,
}

impl RpcRequest {
    #[must_use]
    pub fn new(method: impl Into<String>, params: Value, token: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().simple().to_string(),
            method: method.into(),
            params,
            token: token.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcResponse {
    pub id: String,
    pub result: Option<Value>,
    pub error: Option<RpcError>,
}

impl RpcResponse {
    #[must_use]
    pub fn success(id: impl Into<String>, result: Value) -> Self {
        Self {
            id: id.into(),
            result: Some(result),
            error: None,
        }
    }

    #[must_use]
    pub fn failure(id: impl Into<String>, code: i64, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }

    pub fn into_result(self) -> anyhow::Result<Value> {
        if let Some(error) = self.error {
            bail!("daemon error {}: {}", error.code, error.message)
        }
        self.result
            .context("daemon response did not contain a result")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

#[async_trait]
pub trait RpcHandler: Send + Sync {
    async fn handle(&self, request: RpcRequest) -> RpcResponse;
}

#[derive(Clone, Debug)]
pub struct IpcClient {
    socket_path: PathBuf,
    token: String,
}

impl IpcClient {
    #[must_use]
    pub fn new(socket_path: PathBuf, token: impl Into<String>) -> Self {
        Self {
            socket_path,
            token: token.into(),
        }
    }

    pub fn from_environment() -> anyhow::Result<Self> {
        let socket_path = std::env::var_os("OSANWE_SOCKET")
            .map(PathBuf::from)
            .context("OSANWE_SOCKET is not set")?;
        let token = std::env::var("OSANWE_AGENT_TOKEN")
            .or_else(|_| std::env::var("OSANWE_ADMIN_TOKEN"))
            .context("OSANWE_AGENT_TOKEN or OSANWE_ADMIN_TOKEN is not set")?;
        Ok(Self::new(socket_path, token))
    }

    pub async fn call(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        call_socket(
            &self.socket_path,
            RpcRequest::new(method, params, self.token.clone()),
        )
        .await?
        .into_result()
    }
}

#[cfg(unix)]
pub async fn serve(socket_path: &Path, handler: Arc<dyn RpcHandler>) -> anyhow::Result<()> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    if socket_path.exists() {
        fs::remove_file(socket_path)
            .with_context(|| format!("remove stale socket {}", socket_path.display()))?;
    }
    if let Some(parent) = socket_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("bind {}", socket_path.display()))?;
    fs::set_permissions(socket_path, fs::Permissions::from_mode(0o600))?;

    loop {
        let (stream, _) = listener.accept().await?;
        let handler = handler.clone();
        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let response = match serde_json::from_str::<RpcRequest>(&line) {
                    Ok(request) => handler.handle(request).await,
                    Err(error) => RpcResponse::failure("invalid", -32_700, error.to_string()),
                };
                match serde_json::to_vec(&response) {
                    Ok(mut bytes) => {
                        bytes.push(b'\n');
                        if writer.write_all(&bytes).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

#[cfg(not(unix))]
pub async fn serve(_socket_path: &Path, _handler: Arc<dyn RpcHandler>) -> anyhow::Result<()> {
    bail!("Osanwe IPC currently requires Unix sockets; use Linux, macOS, or WSL")
}

#[cfg(unix)]
async fn call_socket(socket_path: &Path, request: RpcRequest) -> anyhow::Result<RpcResponse> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    let mut stream = UnixStream::connect(socket_path)
        .await
        .with_context(|| format!("connect to {}", socket_path.display()))?;
    let mut payload = serde_json::to_vec(&request)?;
    payload.push(b'\n');
    stream.write_all(&payload).await?;
    stream.shutdown().await?;

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await?;
    if line.is_empty() {
        bail!("daemon closed the connection without a response")
    }
    serde_json::from_str(&line).context("decode daemon response")
}

#[cfg(not(unix))]
async fn call_socket(_socket_path: &Path, _request: RpcRequest) -> anyhow::Result<RpcResponse> {
    bail!("Osanwe IPC currently requires Unix sockets; use Linux, macOS, or WSL")
}
