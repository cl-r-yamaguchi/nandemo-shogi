//! サーバーの設定。環境変数から読み込む。初期値は、ローカルホストだけを許す設定。

use std::net::SocketAddr;

pub const DEFAULT_BIND: &str = "127.0.0.1:8080";
pub const DEFAULT_ALLOWED_ORIGINS: &str = "http://localhost:5173,http://127.0.0.1:5173";
pub const DEFAULT_DB_PATH: &str = "nandemo-shogi.sqlite";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
    pub allowed_origins: Vec<String>,
    pub db_path: String,
}

impl Config {
    /// 環境変数 `NANDEMO_BIND`・`NANDEMO_ALLOWED_ORIGINS`（カンマ区切り）・`NANDEMO_DB_PATH` から読む。
    pub fn from_env() -> Result<Self, String> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let bind = lookup("NANDEMO_BIND").unwrap_or_else(|| DEFAULT_BIND.to_string());
        let bind = bind
            .parse()
            .map_err(|e| format!("NANDEMO_BIND が不正です（{bind}）: {e}"))?;
        let origins = lookup("NANDEMO_ALLOWED_ORIGINS")
            .unwrap_or_else(|| DEFAULT_ALLOWED_ORIGINS.to_string());
        let allowed_origins = origins
            .split(',')
            .map(str::trim)
            .filter(|o| !o.is_empty())
            .map(str::to_string)
            .collect::<Vec<String>>();
        if allowed_origins.is_empty() {
            return Err(
                "NANDEMO_ALLOWED_ORIGINS が空です。許可するオリジンを 1 つ以上指定してください"
                    .to_string(),
            );
        }
        let db_path = lookup("NANDEMO_DB_PATH").unwrap_or_else(|| DEFAULT_DB_PATH.to_string());
        Ok(Self {
            bind,
            allowed_origins,
            db_path,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_listen_on_localhost_only() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert!(config.bind.ip().is_loopback());
        assert_eq!(
            config.allowed_origins,
            ["http://localhost:5173", "http://127.0.0.1:5173"]
        );
    }

    #[test]
    fn overrides_are_applied() {
        let config = Config::from_lookup(|key| match key {
            "NANDEMO_BIND" => Some("0.0.0.0:9000".into()),
            "NANDEMO_ALLOWED_ORIGINS" => Some("https://a.example, https://b.example".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.bind.port(), 9000);
        assert_eq!(
            config.allowed_origins,
            ["https://a.example", "https://b.example"]
        );
    }

    #[test]
    fn empty_origins_are_rejected() {
        let result =
            Config::from_lookup(|key| (key == "NANDEMO_ALLOWED_ORIGINS").then(|| ",".into()));
        assert!(result.is_err());
    }

    #[test]
    fn invalid_bind_is_rejected() {
        assert!(Config::from_lookup(|_| Some("not-an-address".into())).is_err());
    }
}
