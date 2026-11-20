//! A minimal, dependency-light async client for the Telegram Bot API.
//!
//! Built directly on `async-io` + `futures-rustls`, with no runtime lock-in
//! beyond an `async-io`-compatible executor, and no opinion about logging:
//! every fallible call returns a `Result`, and it's up to the caller to log,
//! retry, or propagate it further.
//!
//! ```no_run
//! use telegram_bot_lib::{SendMessageOptions, Telegram};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let bot = Telegram::new("123456:ABC-token".to_string());
//! bot.send_message(42, "hello", SendMessageOptions::new()).await?;
//! # Ok(())
//! # }
//! ```

mod http;
mod telegram;
mod types;

pub use http::HttpError;
pub use telegram::{InputFile, SendMessageOptions, Telegram, TelegramError};
pub use types::{
    CallbackQuery, Chat, Document, InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, Me,
    Message, PhotoSize, ReplyKeyboardMarkup, ReplyKeyboardRemove, ReplyMarkup, Update, User,
};
