use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::http::{self, Body, HttpError, MultipartForm};
use crate::types::envelope::ApiResponse;
use crate::types::{File, Me, ReplyMarkup, Update};

const HOST: &str = "api.telegram.org";
const LONG_POLL_TIMEOUT_SECS: u64 = 30;
/// Give the socket a bit of slack over the long-poll timeout Telegram itself uses,
/// so we don't race it under normal network conditions.
const LONG_POLL_SOCKET_TIMEOUT_SECS: u64 = 35;
const DEFAULT_TIMEOUT_SECS: u64 = 10;
/// Uploads can take longer than a plain text call, especially on a slow
/// connection or for a large document.
const UPLOAD_TIMEOUT_SECS: u64 = 60;

/// Everything that can go wrong when talking to the Telegram Bot API.
///
/// This type carries no text that assumes a particular locale or a particular
/// logging setup: it's plain data for the caller to match on, format, or log
/// however they like.
#[derive(Debug)]
#[non_exhaustive]
pub enum TelegramError {
    /// Failed below the HTTP layer (connect, TLS, timeout, non-2xx status, ...).
    Http(HttpError),
    /// The server replied with a well-formed envelope but `"ok": false`.
    Api {
        error_code: Option<i64>,
        description: String,
    },
    /// Telegram asked us to slow down; wait this many seconds before retrying.
    RateLimited { retry_after: u64 },
    /// The response body wasn't valid JSON, or didn't match the expected shape.
    Json(serde_json::Error),
    /// A local file passed to e.g. [`Telegram::send_photo`] couldn't be read.
    Io(std::io::Error),
}

impl std::fmt::Display for TelegramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TelegramError::Http(e) => write!(f, "transport error: {e}"),
            TelegramError::Api {
                error_code,
                description,
            } => match error_code {
                Some(code) => write!(f, "Telegram API error {code}: {description}"),
                None => write!(f, "Telegram API error: {description}"),
            },
            TelegramError::RateLimited { retry_after } => {
                write!(f, "rate limited, retry after: {retry_after}")
            }
            TelegramError::Json(e) => write!(f, "failed to decode response: {e}"),
            TelegramError::Io(e) => write!(f, "failed to read local file: {e}"),
        }
    }
}

impl std::error::Error for TelegramError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TelegramError::Http(e) => Some(e),
            TelegramError::Api { .. } => None,
            TelegramError::RateLimited { .. } => None,
            TelegramError::Json(e) => Some(e),
            TelegramError::Io(e) => Some(e),
        }
    }
}

impl From<HttpError> for TelegramError {
    fn from(e: HttpError) -> Self {
        TelegramError::Http(e)
    }
}

impl From<serde_json::Error> for TelegramError {
    fn from(e: serde_json::Error) -> Self {
        TelegramError::Json(e)
    }
}

/// Optional extra parameters for [`Telegram::send_message`].
///
/// Constructed with [`SendMessageOptions::new`] and extended with the builder
/// methods, e.g.:
///
/// ```no_run
/// # use telegram_bot_lib::SendMessageOptions;
/// let opts = SendMessageOptions::new()
///     .parse_mode("HTML")
///     .reply_to_message_id(42);
/// ```
#[derive(Debug, Clone, Default)]
pub struct SendMessageOptions {
    parse_mode: Option<String>,
    reply_to_message_id: Option<i64>,
    disable_notification: Option<bool>,
    reply_markup: Option<ReplyMarkup>,
}

impl SendMessageOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn parse_mode(mut self, mode: impl Into<String>) -> Self {
        self.parse_mode = Some(mode.into());
        self
    }

    pub fn reply_to_message_id(mut self, message_id: i64) -> Self {
        self.reply_to_message_id = Some(message_id);
        self
    }

    pub fn disable_notification(mut self, disable: bool) -> Self {
        self.disable_notification = Some(disable);
        self
    }

    /// Attaches an inline keyboard, a reply keyboard, or a keyboard
    /// removal to the message. Accepts anything that converts into
    /// [`ReplyMarkup`] (e.g. an [`crate::InlineKeyboardMarkup`] directly).
    pub fn reply_markup(mut self, markup: impl Into<ReplyMarkup>) -> Self {
        self.reply_markup = Some(markup.into());
        self
    }

    fn apply_to_json(self, payload: &mut serde_json::Value) -> Result<(), TelegramError> {
        if let Some(mode) = self.parse_mode {
            payload["parse_mode"] = serde_json::Value::String(mode);
        }
        if let Some(id) = self.reply_to_message_id {
            payload["reply_to_message_id"] = serde_json::Value::from(id);
        }
        if let Some(disable) = self.disable_notification {
            payload["disable_notification"] = serde_json::Value::from(disable);
        }
        if let Some(markup) = self.reply_markup {
            payload["reply_markup"] = serde_json::to_value(markup)?;
        }
        Ok(())
    }

    /// Same fields as [`Self::apply_to_json`], but flattened onto a
    /// multipart form (used by the file-sending methods).
    fn apply_to_form(self, mut form: MultipartForm) -> Result<MultipartForm, TelegramError> {
        if let Some(mode) = self.parse_mode {
            form = form.field("parse_mode", mode);
        }
        if let Some(id) = self.reply_to_message_id {
            form = form.field("reply_to_message_id", id.to_string());
        }
        if let Some(disable) = self.disable_notification {
            form = form.field("disable_notification", disable.to_string());
        }
        if let Some(markup) = self.reply_markup {
            let json = serde_json::to_string(&markup)?;
            form = form.field("reply_markup", json);
        }
        Ok(form)
    }
}

/// A file to send with [`Telegram::send_photo`] or [`Telegram::send_document`]:
/// either bytes already in memory, or a path to read from disk.
pub enum InputFile<'a> {
    Bytes {
        filename: String,
        mime_type: Option<String>,
        bytes: &'a [u8],
    },
    Path(&'a Path),
}

impl<'a> InputFile<'a> {
    /// Build from raw bytes already in memory (e.g. a generated image).
    pub fn bytes(filename: impl Into<String>, bytes: &'a [u8]) -> Self {
        Self::Bytes {
            filename: filename.into(),
            mime_type: None,
            bytes,
        }
    }

    /// Same as [`Self::bytes`], but with an explicit MIME type instead of
    /// the `application/octet-stream` default.
    pub fn bytes_with_mime(
        filename: impl Into<String>,
        mime_type: impl Into<String>,
        bytes: &'a [u8],
    ) -> Self {
        Self::Bytes {
            filename: filename.into(),
            mime_type: Some(mime_type.into()),
            bytes,
        }
    }

    /// Build from a path on disk; the file is read when the request is sent.
    pub fn path(path: &'a (impl AsRef<Path> + ?Sized)) -> Self {
        Self::Path(path.as_ref())
    }

    fn into_filename_and_bytes(self) -> Result<(String, Option<String>, Vec<u8>), TelegramError> {
        match self {
            InputFile::Bytes {
                filename,
                mime_type,
                bytes,
            } => Ok((filename, mime_type, bytes.to_vec())),
            InputFile::Path(path) => {
                let bytes = std::fs::read(path).map_err(TelegramError::Io)?;
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".to_string());
                Ok((filename, None, bytes))
            }
        }
    }
}

pub struct Telegram {
    token: String,
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

    /// Long-polls for new updates starting after `offset - 1`.
    ///
    /// Blocks (asynchronously) for up to ~30s if there is nothing new,
    /// per the Telegram long-polling convention.
    pub async fn get_updates(&self, offset: i64) -> Result<Vec<Update>, TelegramError> {
        let query = format!("offset={offset}&timeout={LONG_POLL_TIMEOUT_SECS}");
        let path = self.path("getUpdates", &query);

        let response = http::request(
            HOST,
            &path,
            None,
            Duration::from_secs(LONG_POLL_SOCKET_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        decode_api_response(&response.body)
    }

    /// Sends a text message to `chat_id`. Use [`SendMessageOptions`] for
    /// parse mode, replies, silent notifications, or a keyboard.
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        options: SendMessageOptions,
    ) -> Result<(), TelegramError> {
        let path = self.path("sendMessage", "");

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        });
        options.apply_to_json(&mut payload)?;

        let body = serde_json::to_vec(&payload)?;

        let response = http::request(
            HOST,
            &path,
            Some(body),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        let _: serde_json::Value = decode_api_response(&response.body)?;
        Ok(())
    }

    /// Sends a photo, with an optional caption, to `chat_id`.
    ///
    /// `caption` is sent as the `caption` field (not `text`); apply
    /// [`SendMessageOptions::parse_mode`] to it the same way as for
    /// [`Self::send_message`].
    pub async fn send_photo(
        &self,
        chat_id: i64,
        photo: InputFile<'_>,
        caption: Option<&str>,
        options: SendMessageOptions,
    ) -> Result<(), TelegramError> {
        let (filename, mime_type, bytes) = photo.into_filename_and_bytes()?;

        let mut form = MultipartForm::new()
            .field("chat_id", chat_id.to_string())
            .file("photo", &filename, mime_type.as_deref(), &bytes);
        if let Some(caption) = caption {
            form = form.field("caption", caption);
        }
        let form = options.apply_to_form(form)?;

        self.send_multipart("sendPhoto", form).await
    }

    /// Sends an arbitrary file as a document to `chat_id`.
    pub async fn send_document(
        &self,
        chat_id: i64,
        document: InputFile<'_>,
        caption: Option<&str>,
        options: SendMessageOptions,
    ) -> Result<(), TelegramError> {
        let (filename, mime_type, bytes) = document.into_filename_and_bytes()?;

        let mut form = MultipartForm::new()
            .field("chat_id", chat_id.to_string())
            .file("document", &filename, mime_type.as_deref(), &bytes);
        if let Some(caption) = caption {
            form = form.field("caption", caption);
        }
        let form = options.apply_to_form(form)?;

        self.send_multipart("sendDocument", form).await
    }

    async fn send_multipart(&self, method: &str, form: MultipartForm) -> Result<(), TelegramError> {
        let path = self.path(method, "");

        let response = http::request_with_body(
            HOST,
            &path,
            Some(Body::Multipart(form)),
            Duration::from_secs(UPLOAD_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        let _: serde_json::Value = decode_api_response(&response.body)?;
        Ok(())
    }

    /// Acknowledges an inline-keyboard button press. Telegram keeps the
    /// button's loading spinner showing until this is called, so call it
    /// (even with no `text`) for every [`crate::CallbackQuery`] you handle.
    ///
    /// `text`, when given, pops up as a small notification (or an alert
    /// dialog if `show_alert` is `true`) on the user's client; it is not
    /// posted as a chat message.
    pub async fn answer_callback_query(
        &self,
        callback_query_id: &str,
        text: Option<&str>,
        show_alert: bool,
    ) -> Result<(), TelegramError> {
        let path = self.path("answerCallbackQuery", "");

        let mut payload = serde_json::json!({
            "callback_query_id": callback_query_id,
        });
        if let Some(text) = text {
            payload["text"] = serde_json::Value::String(text.to_string());
        }
        if show_alert {
            payload["show_alert"] = serde_json::Value::Bool(true);
        }

        let body = serde_json::to_vec(&payload)?;
        let response = http::request(
            HOST,
            &path,
            Some(body),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        let _: serde_json::Value = decode_api_response(&response.body)?;
        Ok(())
    }

    /// Edits the inline keyboard on a message that was sent with one
    /// (e.g. to show a new state after a button press), without touching
    /// its text.
    pub async fn edit_message_reply_markup(
        &self,
        chat_id: i64,
        message_id: i64,
        markup: Option<crate::InlineKeyboardMarkup>,
    ) -> Result<(), TelegramError> {
        let path = self.path("editMessageReplyMarkup", "");

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
        });
        if let Some(markup) = markup {
            payload["reply_markup"] = serde_json::to_value(markup)?;
        }

        let body = serde_json::to_vec(&payload)?;
        let response = http::request(
            HOST,
            &path,
            Some(body),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        let _: serde_json::Value = decode_api_response(&response.body)?;
        Ok(())
    }

    /// Calls `getMe`, mostly useful to verify the token works and to learn
    /// the bot's own username.
    pub async fn get_me(&self) -> Result<Me, TelegramError> {
        let path = self.path("getMe", "");

        let response = http::request(HOST, &path, None, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .await
            .map_err(classify_http_error)?;

        decode_api_response(&response.body)
    }

    /// Deletes a message. Telegram only allows this within 48 hours for most
    /// chat types, and the bot needs admin rights in groups/channels to delete
    /// messages it didn't send itself.
    pub async fn delete_message(&self, chat_id: i64, message_id: i64) -> Result<(), TelegramError> {
        let path = self.path("deleteMessage", "");

        let payload = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
        });

        let body = serde_json::to_vec(&payload)?;
        let response = http::request(
            HOST,
            &path,
            Some(body),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        let _: serde_json::Value = decode_api_response(&response.body)?;
        Ok(())
    }

    /// Edits the text of a previously sent message. Only `parse_mode` and
    /// `reply_markup` from `options` apply here; Telegram ignores the rest for
    /// this method.
    pub async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        options: SendMessageOptions,
    ) -> Result<(), TelegramError> {
        let path = self.path("editMessageText", "");

        let mut payload = serde_json::json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
        });
        options.apply_to_json(&mut payload)?;

        let body = serde_json::to_vec(&payload)?;
        let response = http::request(
            HOST,
            &path,
            Some(body),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        let _: serde_json::Value = decode_api_response(&response.body)?;
        Ok(())
    }

    /// Shows a transient status indicator ("typing...", "uploading photo...")
    /// in the chat. Telegram clears it automatically after ~5s or on the next
    /// message, whichever comes first — call it again if an operation takes
    /// longer.
    ///
    /// Common values for `action`: `"typing"`, `"upload_photo"`,
    /// `"record_video"`, `"upload_video"`, `"upload_document"`,
    /// `"choose_sticker"`, `"find_location"`.
    pub async fn send_chat_action(&self, chat_id: i64, action: &str) -> Result<(), TelegramError> {
        let path = self.path("sendChatAction", "");

        let payload = serde_json::json!({
            "chat_id": chat_id,
            "action": action,
        });

        let body = serde_json::to_vec(&payload)?;
        let response = http::request(
            HOST,
            &path,
            Some(body),
            Duration::from_secs(DEFAULT_TIMEOUT_SECS),
        )
        .await
        .map_err(classify_http_error)?;

        let _: serde_json::Value = decode_api_response(&response.body)?;
        Ok(())
    }

    /// Calls `getFile` to resolve a `file_id` into a [`File`] with a
    /// `file_path`. Note: `file_path` is only valid for ~1 hour after this
    /// call, and Telegram imposes a 20 MB cap on files retrievable this way
    /// regardless of the file's actual size limit for uploads.
    pub async fn get_file(&self, file_id: &str) -> Result<File, TelegramError> {
        let query = format!("file_id={file_id}");
        let path = self.path("getFile", &query);

        let response = http::request(HOST, &path, None, Duration::from_secs(DEFAULT_TIMEOUT_SECS))
            .await
            .map_err(classify_http_error)?;

        decode_api_response(&response.body)
    }

    /// Downloads the raw bytes of a file previously resolved with
    /// [`Self::get_file`]. This hits a different host
    /// (`api.telegram.org/file/...`) than the rest of the Bot API, so it
    /// does not go through [`Self::path`].
    pub async fn download_file(&self, file_path: &str) -> Result<Vec<u8>, TelegramError> {
        let path = format!("/file/bot{}/{}", self.token, file_path);

        let response = http::request(HOST, &path, None, Duration::from_secs(UPLOAD_TIMEOUT_SECS))
            .await
            .map_err(classify_http_error)?;

        Ok(response.body)
    }

    /// Convenience: resolves `file_id` and downloads it in one call.
    pub async fn download(&self, file_id: &str) -> Result<Vec<u8>, TelegramError> {
        let file = self.get_file(file_id).await?;
        let file_path = file.file_path.ok_or_else(|| TelegramError::Api {
            error_code: None,
            description: "getFile returned no file_path".to_string(),
        })?;
        self.download_file(&file_path).await
    }
}

/// Decodes a Telegram API envelope `{"ok": ..., "result": ..., ...}` into `T`,
/// turning `"ok": false` into [`TelegramError::Api`] and a missing `result`
/// on an `"ok": true` response into [`TelegramError::Api`] as well, rather
/// than silently manufacturing a default value.
fn decode_api_response<T>(body: &[u8]) -> Result<T, TelegramError>
where
    T: for<'de> Deserialize<'de>,
{
    let parsed: ApiResponse<T> = serde_json::from_slice(body)?;
    if !parsed.ok {
        return Err(TelegramError::Api {
            error_code: parsed.error_code,
            description: parsed
                .description
                .unwrap_or_else(|| "unknown error".to_string()),
        });
    }
    parsed.result.ok_or_else(|| TelegramError::Api {
        error_code: None,
        description: "server reported success but sent no result".to_string(),
    })
}

fn classify_http_error(err: HttpError) -> TelegramError {
    if let HttpError::Status(429, ref body) = err {
        if let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(body) {
            if let Some(retry_after) = parsed
                .get("parameters")
                .and_then(|p| p.get("retry_after"))
                .and_then(|v| v.as_u64())
            {
                return TelegramError::RateLimited { retry_after };
            }
        }
    }
    TelegramError::Http(err)
}
