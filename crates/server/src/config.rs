//! サーバーの設定。環境変数から読み込む。初期値は、ローカルホストだけを許す設定。

use std::net::SocketAddr;

pub const DEFAULT_BIND: &str = "127.0.0.1:8080";
pub const DEFAULT_ALLOWED_ORIGINS: &str = "http://localhost:5173,http://127.0.0.1:5173";
pub const DEFAULT_DB_PATH: &str = "nandemo-shogi.sqlite";
pub const DEFAULT_MAX_CONNECTIONS: usize = 100;
/// AI の手を、見せるまでの最低の間（ミリ秒）。何が起きたか分かるように、少し待つ（ui.md の 4.4）。
pub const DEFAULT_AI_DELAY_MS: u64 = 1500;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub bind: SocketAddr,
    pub allowed_origins: Vec<String>,
    pub db_path: String,
    /// 同時に受け付ける接続の上限。
    pub max_connections: usize,
    /// AI の手を反映するまでの、最低の間（ミリ秒）。
    pub ai_delay_ms: u64,
}

impl Config {
    /// 環境変数 `NANDEMO_BIND`・`NANDEMO_ALLOWED_ORIGINS`（カンマ区切り）・`NANDEMO_DB_PATH`・
    /// `NANDEMO_MAX_CONNECTIONS`・`NANDEMO_AI_DELAY_MS` から読む。
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
        let max_connections =
            parse_number(&lookup, "NANDEMO_MAX_CONNECTIONS", DEFAULT_MAX_CONNECTIONS)?;
        if max_connections == 0 {
            return Err("NANDEMO_MAX_CONNECTIONS は、1 以上にしてください".to_string());
        }
        let ai_delay_ms = parse_number(&lookup, "NANDEMO_AI_DELAY_MS", DEFAULT_AI_DELAY_MS)?;
        Ok(Self {
            bind,
            allowed_origins,
            db_path,
            max_connections,
            ai_delay_ms,
        })
    }
}

impl Config {
    /// テスト用: 許可するオリジンを 1 つだけ指定した設定。
    #[cfg(test)]
    pub(crate) fn from_lookup_for_tests(origin: &str) -> Self {
        let origin = origin.to_string();
        Self::from_lookup(|key| (key == "NANDEMO_ALLOWED_ORIGINS").then(|| origin.clone()))
            .expect("テスト用の設定は、正しい")
    }
}

/// 環境変数の数値を読む。指定がなければ既定値。数値でなければ、エラー。
fn parse_number<T: std::str::FromStr>(
    lookup: &impl Fn(&str) -> Option<String>,
    key: &str,
    default: T,
) -> Result<T, String> {
    match lookup(key) {
        None => Ok(default),
        Some(value) => value
            .trim()
            .parse()
            .map_err(|_| format!("{key} が不正です（{value}）。数値を指定してください")),
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

    #[test]
    fn connection_limit_and_ai_delay_have_defaults_and_can_be_overridden() {
        let config = Config::from_lookup(|_| None).unwrap();
        assert_eq!(config.max_connections, DEFAULT_MAX_CONNECTIONS);
        assert_eq!(config.ai_delay_ms, DEFAULT_AI_DELAY_MS);
        let config = Config::from_lookup(|key| match key {
            "NANDEMO_MAX_CONNECTIONS" => Some("5".into()),
            "NANDEMO_AI_DELAY_MS" => Some("0".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!((config.max_connections, config.ai_delay_ms), (5, 0));
    }

    #[test]
    fn non_numeric_limits_are_rejected() {
        let result =
            Config::from_lookup(|key| (key == "NANDEMO_MAX_CONNECTIONS").then(|| "many".into()));
        assert!(result.is_err());
    }

    #[test]
    fn a_zero_connection_limit_is_rejected() {
        let result =
            Config::from_lookup(|key| (key == "NANDEMO_MAX_CONNECTIONS").then(|| "0".into()));
        assert!(result.is_err());
    }
}
