use serde::{Deserialize, Serialize};

/// A Telegram update, as returned by `getUpdates`.
///
/// Only the fields needed for a simple text-message bot (plus buttons) are
/// modeled here. Other update kinds simply deserialize with the relevant
/// field set to `None`.
#[derive(Debug, Clone, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub message_id: i64,
    pub date: i64,
    pub chat: Chat,
    pub from: Option<User>,
    pub text: Option<String>,
    /// Present when the message is a photo; Telegram sends several sizes,
    /// largest last.
    #[serde(default)]
    pub photo: Option<Vec<PhotoSize>>,
    #[serde(default)]
    pub document: Option<Document>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PhotoSize {
    pub file_id: String,
    pub file_unique_id: String,
    pub width: i64,
    pub height: i64,
    #[serde(default)]
    pub file_size: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Document {
    pub file_id: String,
    pub file_unique_id: String,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub file_size: Option<i64>,
}

/// A press of an inline keyboard button.
///
/// `data` is whatever you put in [`InlineKeyboardButton::callback`]. Telegram
/// requires every callback query to be acknowledged (see
/// [`crate::Telegram::answer_callback_query`]), or the user's client will
/// show a loading spinner on the button for a while.
#[derive(Debug, Clone, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub data: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Chat {
    pub id: i64,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub username: Option<String>,
}

/// The result of a `getMe` call.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Me {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    pub username: Option<String>,
}

/// One button in an inline keyboard (attached to a message, not the
/// on-screen keyboard). Build rows with [`InlineKeyboardMarkup::new`].
#[derive(Debug, Clone, Serialize)]
pub struct InlineKeyboardButton {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl InlineKeyboardButton {
    /// A button that sends `data` back to your bot in a [`CallbackQuery`]
    /// when pressed. Telegram limits `data` to 64 bytes.
    pub fn callback(text: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            callback_data: Some(data.into()),
            url: None,
        }
    }

    /// A button that opens `url` in the user's browser when pressed.
    pub fn url(text: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            callback_data: None,
            url: Some(url.into()),
        }
    }
}

/// An inline keyboard: buttons attached under a specific message.
///
/// ```no_run
/// # use telegram_bot_lib::{InlineKeyboardButton, InlineKeyboardMarkup};
/// let kb = InlineKeyboardMarkup::new(vec![
///     vec![
///         InlineKeyboardButton::callback("Так", "confirm:yes"),
///         InlineKeyboardButton::callback("Ні", "confirm:no"),
///     ],
/// ]);
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct InlineKeyboardMarkup {
    pub inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

impl InlineKeyboardMarkup {
    pub fn new(rows: Vec<Vec<InlineKeyboardButton>>) -> Self {
        Self {
            inline_keyboard: rows,
        }
    }

    /// Convenience for a single row of buttons.
    pub fn row(buttons: Vec<InlineKeyboardButton>) -> Self {
        Self::new(vec![buttons])
    }
}

/// One button on the reply keyboard (replaces the user's on-screen
/// keyboard, as opposed to [`InlineKeyboardMarkup`] which attaches to a
/// message).
#[derive(Debug, Clone, Serialize)]
pub struct KeyboardButton {
    pub text: String,
}

impl KeyboardButton {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

impl From<&str> for KeyboardButton {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

/// The on-screen keyboard shown instead of the regular one. Send it once;
/// Telegram keeps showing it until you send [`ReplyKeyboardRemove`] or
/// another markup.
///
/// ```no_run
/// # use telegram_bot_lib::ReplyKeyboardMarkup;
/// let kb = ReplyKeyboardMarkup::new(vec![
///     vec!["🔥 Факт".into(), "ℹ️ Допомога".into()],
/// ])
/// .resize_keyboard(true);
/// ```
#[derive(Debug, Clone, Serialize)]
pub struct ReplyKeyboardMarkup {
    pub keyboard: Vec<Vec<KeyboardButton>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resize_keyboard: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_time_keyboard: Option<bool>,
}

impl ReplyKeyboardMarkup {
    pub fn new(rows: Vec<Vec<KeyboardButton>>) -> Self {
        Self {
            keyboard: rows,
            resize_keyboard: None,
            one_time_keyboard: None,
        }
    }

    /// Ask clients to shrink the keyboard to fit the buttons (otherwise it
    /// takes up as much vertical space as the system keyboard).
    pub fn resize_keyboard(mut self, resize: bool) -> Self {
        self.resize_keyboard = Some(resize);
        self
    }

    /// Hide the keyboard again after one use.
    pub fn one_time_keyboard(mut self, one_time: bool) -> Self {
        self.one_time_keyboard = Some(one_time);
        self
    }
}

/// Sent in place of a keyboard markup to remove the current reply keyboard.
#[derive(Debug, Clone, Serialize)]
pub struct ReplyKeyboardRemove {
    pub remove_keyboard: bool,
}

impl Default for ReplyKeyboardRemove {
    fn default() -> Self {
        Self {
            remove_keyboard: true,
        }
    }
}

/// Either kind of keyboard (or a removal), as attachable to a sent message.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ReplyMarkup {
    Inline(InlineKeyboardMarkup),
    Reply(ReplyKeyboardMarkup),
    Remove(ReplyKeyboardRemove),
}

impl From<InlineKeyboardMarkup> for ReplyMarkup {
    fn from(kb: InlineKeyboardMarkup) -> Self {
        Self::Inline(kb)
    }
}

impl From<ReplyKeyboardMarkup> for ReplyMarkup {
    fn from(kb: ReplyKeyboardMarkup) -> Self {
        Self::Reply(kb)
    }
}

impl From<ReplyKeyboardRemove> for ReplyMarkup {
    fn from(kb: ReplyKeyboardRemove) -> Self {
        Self::Remove(kb)
    }
}

pub(crate) mod envelope {
    use serde::Deserialize;

    /// Every Telegram Bot API response has this shape, whatever `T` is.
    #[derive(Deserialize)]
    pub struct ApiResponse<T> {
        pub ok: bool,
        pub result: Option<T>,
        pub error_code: Option<i64>,
        pub description: Option<String>,
    }
}
