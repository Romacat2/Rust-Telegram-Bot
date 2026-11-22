mod log;

use std::sync::Arc;
use std::time::Duration;

use async_executor::Executor;
use telegram_bot_lite::{
    CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, KeyboardButton, Message,
    ReplyKeyboardMarkup, SendMessageOptions, Telegram, Update,
};

fn main() {
    dotenvy::dotenv().ok();

    log::init();
    log::info("Бот запускається...");

    let token = std::env::var("TELEGRAM_TOKEN").expect("Встанови TELEGRAM_TOKEN у файлі .env");
    let bot = Arc::new(Telegram::new(token));

    let ex: Executor<'static> = Executor::new();

    async_io::block_on(ex.run(async {
        // The library can tell us right away if the token is bad, instead of
        // only failing on the first getUpdates call.
        match bot.get_me().await {
            Ok(me) => log::info(&format!(
                "Авторизовано як @{}",
                me.username.as_deref().unwrap_or(&me.first_name)
            )),
            Err(e) => {
                log::error(&format!("Не вдалося авторизуватись у Telegram: {e}"));
                return;
            }
        }

        log::info("Бот запущено, чекаю повідомлення...");
        poll_loop(&ex, bot).await;
    }));
}

async fn poll_loop(ex: &Executor<'static>, bot: Arc<Telegram>) {
    let mut offset: i64 = 0;

    loop {
        match bot.get_updates(offset).await {
            Ok(updates) => {
                for update in updates {
                    offset = update.update_id + 1;

                    let bot = Arc::clone(&bot);
                    ex.spawn(async move {
                        handle_update(&bot, update).await;
                    })
                    .detach();
                }
            }
            Err(e) => {
                log::error(&format!("Помилка отримання оновлень: {e}"));
                async_io::Timer::after(Duration::from_secs(3)).await;
            }
        }
    }
}

async fn handle_update(bot: &Telegram, update: Update) {
    if let Some(callback) = update.callback_query {
        handle_callback_query(bot, callback).await;
        return;
    }

    let Some(message) = update.message else {
        return;
    };
    handle_message(bot, message).await;
}

/// The bottom reply keyboard, shown after `/start`. Buttons on it just send
/// their label back as plain text, so they're handled in [`handle_message`]
/// alongside the slash commands.
fn main_keyboard() -> ReplyKeyboardMarkup {
    ReplyKeyboardMarkup::new(vec![
        vec![
            KeyboardButton::new("🔥 Факт"),
            KeyboardButton::new("ℹ️ Допомога"),
        ],
        vec![KeyboardButton::new("🖼️ Картинка")],
    ])
    .resize_keyboard(true)
}

/// The inline keyboard attached to `/help` — buttons live on that specific
/// message and report back through a `callback_query`, not a new message.
fn help_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("🔥 Факт", "fact"),
            InlineKeyboardButton::callback("🖼️ Картинка", "photo"),
        ],
        vec![InlineKeyboardButton::url(
            "Документація Telegram Bot API",
            "https://core.telegram.org/bots/api",
        )],
    ])
}

async fn handle_message(bot: &Telegram, message: Message) {
    let chat_id = message.chat.id;

    if let Some(photo) = &message.photo {
        // Telegram sends several sizes, largest last.
        if let Some(largest) = photo.last() {
            log::info(&format!(
                "Отримано фото від chat_id={chat_id}, {}x{}",
                largest.width, largest.height
            ));

            let _ = bot.send_chat_action(chat_id, "typing").await;

            match bot.download(&largest.file_id).await {
                Ok(bytes) => {
                    let path = format!("downloads/{}.jpg", largest.file_unique_id);
                    if let Err(e) = std::fs::create_dir_all("downloads")
                        .and_then(|_| std::fs::write(&path, &bytes))
                    {
                        log::error(&format!("Не вдалося зберегти фото: {e}"));
                    } else {
                        log::info(&format!("Фото збережено: {path} ({} байт)", bytes.len()));
                        let _ = bot
                            .send_message(
                                chat_id,
                                "Гарне фото! 📸 Збережено.",
                                SendMessageOptions::new(),
                            )
                            .await;
                    }
                }
                Err(e) => {
                    log::error(&format!("Не вдалося завантажити фото: {e}"));
                    let _ = bot
                        .send_message(
                            chat_id,
                            "Не вдалося обробити фото 😕",
                            SendMessageOptions::new(),
                        )
                        .await;
                }
            }
        }
        return;
    }

    let Some(text) = message.text else {
        return;
    };

    log::info(&format!("Повідомлення \"{text}\" від chat_id={chat_id}"));

    match text.trim() {
        "/start" => {
            let reply = "Привіт! Я простий бот на Rust 🦀\nНапиши /help, щоб побачити команди, \
                         або скористайся кнопками внизу.";
            let _ = bot
                .send_message(
                    chat_id,
                    reply,
                    SendMessageOptions::new().reply_markup(main_keyboard()),
                )
                .await;
        }
        "/help" | "ℹ️ Допомога" => {
            let reply = "Доступні команди:\n\
                         /start — почати роботу з ботом\n\
                         /help — показати це повідомлення\n\
                         /fact — показати випадковий факт\n\
                         /photo — надіслати картинку";
            if let Err(e) = bot
                .send_message(
                    chat_id,
                    reply,
                    SendMessageOptions::new().reply_markup(help_keyboard()),
                )
                .await
            {
                log::error(&format!("Не вдалося відправити /help: {e}"));
            }
        }
        "/clear" => {
            if let Err(e) = bot.delete_message(chat_id, message.message_id).await {
                log::error(&format!("Не вдалося видалити повідомлення: {e}"));
                let _ = bot
                    .send_message(
                        chat_id,
                        "Не можу видалити повідомлення (можливо, бракує прав адміністратора).",
                        SendMessageOptions::new(),
                    )
                    .await;
            }
        }
        "/fact" | "🔥 Факт" => send_fact(bot, chat_id, None).await,
        "/photo" | "🖼️ Картинка" => send_rust_photo(bot, chat_id, None).await,
        _ => {}
    }
}

async fn handle_callback_query(bot: &Telegram, callback: CallbackQuery) {
    let Some(data) = callback.data.as_deref() else {
        let _ = bot.answer_callback_query(&callback.id, None, false).await;
        return;
    };
    let chat_id = callback.message.as_ref().map(|m| m.chat.id);

    log::info(&format!(
        "Натиснуто кнопку \"{data}\" від user_id={}",
        callback.from.id
    ));

    match data {
        "fact" => {
            // Acknowledge first so the button's spinner clears right away;
            // the fact itself arrives as a normal message.
            let _ = bot.answer_callback_query(&callback.id, None, false).await;
            if let Some(chat_id) = chat_id {
                send_fact(bot, chat_id, None).await;
            }
        }
        "photo" => {
            let _ = bot
                .answer_callback_query(&callback.id, Some("Надсилаю картинку…"), false)
                .await;
            if let Some(chat_id) = chat_id {
                send_rust_photo(bot, chat_id, None).await;
            }
        }
        "another_fact" => {
            let _ = bot.answer_callback_query(&callback.id, None, false).await;
            if let Some(message) = &callback.message {
                let text = FACTS[rand_index()];
                if let Err(e) = bot
                    .edit_message_text(
                        message.chat.id,
                        message.message_id,
                        text,
                        SendMessageOptions::new().reply_markup(fact_keyboard()),
                    )
                    .await
                {
                    log::error(&format!("Не вдалося відредагувати факт: {e}"));
                }
            }
        }
        other => {
            let _ = bot
                .answer_callback_query(&callback.id, Some(&format!("Невідома дія: {other}")), true)
                .await;
        }
    }
}

fn fact_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::row(vec![InlineKeyboardButton::callback(
        "🔄 Ще факт",
        "another_fact",
    )])
}

const FACTS: &[&str] = &[
    "Цікавий факт: Rust вже 9 років поспіль займає перше місце як \"найулюбленіша мова програмування\" у StackOverflow Survey.",
    "Цікавий факт: компілятор Rust спершу був написаний на OCaml.",
    "Цікавий факт: назва \"Rust\" походить від грибка-іржі (rust fungi), який є \"дуже стійким\".",
];

async fn send_fact(bot: &Telegram, chat_id: i64, reply_to: Option<i64>) {
    let text = FACTS[rand_index()];

    let mut options = SendMessageOptions::new().reply_markup(fact_keyboard());
    if let Some(id) = reply_to {
        options = options.reply_to_message_id(id);
    }

    if let Err(e) = bot.send_message(chat_id, text, options).await {
        log::error(&format!("Не вдалося відправити факт: {e}"));
    }
}

fn rand_index() -> usize {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (nanos as usize) % FACTS.len()
}

/// Sends a small Rust logo PNG, to demonstrate `send_photo`
async fn send_rust_photo(bot: &Telegram, chat_id: i64, reply_to: Option<i64>) {
    let _ = bot.send_chat_action(chat_id, "upload_photo").await;

    let mut options = SendMessageOptions::new();
    if let Some(id) = reply_to {
        options = options.reply_to_message_id(id);
    }

    let photo = InputFile::path("image/rust.png");
    if let Err(e) = bot
        .send_photo(
            chat_id,
            photo,
            Some("Ось картинка логотипу Rust 🦀"),
            options,
        )
        .await
    {
        log::error(&format!("Не вдалося відправити фото: {e}"));
    }
}
