use std::time::Duration;

use serde::Deserialize;

use crate::http;

const HOST: &str = "api.telegram.org";

pub struct Telegram {
    token: String,
}

#[derive(Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

#[derive(Deserialize)]
pub struct Message {
    pub text: Option<String>,
    pub chat: Chat,
}

#[derive(Deserialize)]
pub struct Chat {
    pub id: i64,
}

#[derive(Deserialize)]
struct ApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

impl Telegram {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    fn path(&self, method: &str, query: &str) -> String {
        if query.is_empty() {
            format!("/bot{}/{}", self.token, method)
        } else {
            format!("/bot{}/{}?{}", self.token, method, query)
        }
    }

    pub async fn get_updates(&self, offset: i64) -> Result<Vec<Update>, String> {
        let query = format!("offset={offset}&timeout=30");
        let path = self.path("getUpdates", &query);

        let response = http::request(HOST, &path, None, Duration::from_secs(35))
            .await
            .map_err(|e| e.to_string())?;

        parse_api_response(&response.body)
    }

    pub async fn send_message(&self, chat_id: i64, text: &str) -> Result<(), String> {
        let path = self.path("sendMessage", "");

        let payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        });
        let body = serde_json::to_vec(&payload).map_err(|e| e.to_string())?;

        let response = http::request(HOST, &path, Some(body), Duration::from_secs(10))
            .await
            .map_err(|e| e.to_string())?;

        let _: serde_json::Value = parse_api_response(&response.body)?;
        Ok(())
    }
}

fn parse_api_response<T: for<'de> Deserialize<'de>>(body: &[u8]) -> Result<T, String>
where
    T: Default,
{
    let parsed: ApiResponse<T> = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    if !parsed.ok {
        return Err(parsed.description.unwrap_or_else(|| "unknown error".into()));
    }
    Ok(parsed.result.unwrap_or_default())
}
