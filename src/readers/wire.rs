use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JsonlRecord {
    #[serde(rename = "type")]
    pub record_type: Option<String>,
    #[serde(default)]
    pub is_sidechain: bool,
    pub message: Option<JsonlMessage>,
    #[serde(rename = "costUSD")]
    pub cost_usd: Option<f64>,
    pub timestamp: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub entrypoint: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JsonlMessage {
    pub id: Option<String>,
    pub model: Option<String>,
    pub usage: Option<JsonlUsage>,
    pub content: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
pub struct JsonlUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
}
