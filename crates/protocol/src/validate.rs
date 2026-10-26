//! 受信メッセージの検証（サイズ・形・版・値）。仕様は online.md の 6.4。

use std::fmt;

use crate::messages::{ClientFrame, ClientMessage, RejectCode, ServerFrame, ServerMessage};
use crate::{
    MAX_AI_LEVEL, MAX_DISPLAY_NAME_CHARS, MAX_MESSAGE_BYTES, MAX_PLAYERS, MIN_AI_LEVEL,
    MIN_PLAYERS, PLAYER_ID_LEN, PROTOCOL_VERSION,
};

/// 値の検証のエラー。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldError {
    DisplayName,
    PlayerId,
    Rules,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DisplayName => write!(
                f,
                "表示名は、{MAX_DISPLAY_NAME_CHARS} 文字以内で、制御文字や見えない文字を含められません"
            ),
            Self::PlayerId => f.write_str("プレイヤー ID が不正です"),
            Self::Rules => write!(
                f,
                "ルールの値が範囲外です（人数 {MIN_PLAYERS}〜{MAX_PLAYERS}、AI の強さ {MIN_AI_LEVEL}〜{MAX_AI_LEVEL}）"
            ),
        }
    }
}

impl std::error::Error for FieldError {}

/// 受信メッセージを読み取れなかった理由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooLarge {
        bytes: usize,
    },
    Malformed(String),
    /// 受信した版（数値の文字列。範囲外・小数でも、そのまま）。
    VersionMismatch {
        got: String,
    },
    Field(FieldError),
}

impl DecodeError {
    /// 返事（`rejected`）に使う理由。
    pub fn reject_code(&self) -> RejectCode {
        match self {
            Self::TooLarge { .. } => RejectCode::TooLarge,
            Self::Malformed(_) => RejectCode::BadMessage,
            Self::VersionMismatch { .. } => RejectCode::VersionMismatch,
            Self::Field(FieldError::DisplayName) => RejectCode::InvalidName,
            Self::Field(FieldError::PlayerId) => RejectCode::InvalidPlayerId,
            Self::Field(FieldError::Rules) => RejectCode::InvalidRules,
        }
    }

    /// 返事に添える、人が読む文言。
    pub fn message(&self) -> String {
        match self {
            Self::TooLarge { bytes } => {
                format!("メッセージが大きすぎます（{bytes} バイト。上限 {MAX_MESSAGE_BYTES}）")
            }
            Self::Malformed(reason) => format!("メッセージを読み取れません: {reason}"),
            Self::VersionMismatch { got } => {
                format!("プロトコルの版が合いません（受信 {got}、対応 {PROTOCOL_VERSION}）")
            }
            Self::Field(error) => error.to_string(),
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for DecodeError {}

/// クライアントからのメッセージを読み取り、検証する。
pub fn decode_client(text: &str) -> Result<ClientFrame, DecodeError> {
    if text.len() > MAX_MESSAGE_BYTES {
        return Err(DecodeError::TooLarge { bytes: text.len() });
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| DecodeError::Malformed(e.to_string()))?;
    // 版を先に確かめる。版が違うクライアントには、ほかの項目が読めなくても、版の不一致を返す
    match value.get("v") {
        Some(serde_json::Value::Number(v)) if v.as_u64() == Some(u64::from(PROTOCOL_VERSION)) => {}
        Some(serde_json::Value::Number(v)) => {
            return Err(DecodeError::VersionMismatch { got: v.to_string() });
        }
        _ => {
            return Err(DecodeError::Malformed(
                "版（v）が、数値で指定されていません".into(),
            ));
        }
    }
    let mut frame: ClientFrame =
        serde_json::from_value(value).map_err(|e| DecodeError::Malformed(e.to_string()))?;
    match &mut frame.message {
        ClientMessage::Hello {
            player_id,
            display_name,
        } => {
            validate_player_id(player_id).map_err(DecodeError::Field)?;
            *display_name = validate_display_name(display_name).map_err(DecodeError::Field)?;
        }
        ClientMessage::FindRoom { rules } => {
            if !rules.is_valid() {
                return Err(DecodeError::Field(FieldError::Rules));
            }
        }
        ClientMessage::LeaveRoom
        | ClientMessage::Start
        | ClientMessage::Act { .. }
        | ClientMessage::Resign => {}
    }
    Ok(frame)
}

/// サーバーのメッセージを、送る文字列にする。
pub fn encode_server(frame: &ServerFrame) -> String {
    // 型が `Serialize` を満たし、文字列のキーだけなので、失敗しない
    serde_json::to_string(frame).expect("サーバーのメッセージは、JSON にできる")
}

/// 返事（`rejected`）のメッセージを作る。
impl ServerMessage {
    pub fn rejected(code: RejectCode, message: impl Into<String>) -> Self {
        Self::Rejected {
            code,
            reason: None,
            message: message.into(),
        }
    }

    /// 規則に反する行動の拒否。文言は、理由の説明。
    pub fn illegal_action(reason: rules::action::ActionError) -> Self {
        Self::Rejected {
            code: RejectCode::IllegalAction,
            reason: Some(reason),
            message: reason.to_string(),
        }
    }
}

/// 表示名: 前後の空白を除いて、1〜12 文字（文字数で数える）。制御文字を含められない。
/// 検証した（空白を除いた）名前を返す。
pub fn validate_display_name(name: &str) -> Result<String, FieldError> {
    let name = name.trim();
    let chars = name.chars().count();
    if chars == 0
        || chars > MAX_DISPLAY_NAME_CHARS
        || name.chars().any(|c| c.is_control() || is_invisible(c))
    {
        return Err(FieldError::DisplayName);
    }
    Ok(name.to_string())
}

/// 見えない・表示の向きを変える文字（ゼロ幅の空白、向きの制御、BOM など）。
/// 名前が見えなくなったり、周りの文字の並びを乱したりして、なりすましに使えるため、使えない。
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'                       // ソフトハイフン
            | '\u{061C}'                 // アラビア文字の向きの印
            | '\u{180E}'                 // モンゴル語の母音区切り
            | '\u{200B}'..='\u{200F}'    // ゼロ幅の空白・つなぎ・向きの印
            | '\u{202A}'..='\u{202E}'    // 向きの埋め込み・上書き
            | '\u{2060}'..='\u{206F}'    // 単語の結合子・見えない演算子・向きの分離
            | '\u{FEFF}'                 // BOM（ゼロ幅の改行しない空白）
            | '\u{FFF9}'..='\u{FFFB}'    // 注釈の記号
    )
}

/// プレイヤー ID: 16〜64 文字の、英数字・`-`・`_`。
pub fn validate_player_id(id: &str) -> Result<(), FieldError> {
    let ok = PLAYER_ID_LEN.contains(&id.chars().count())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(FieldError::PlayerId)
    }
}
