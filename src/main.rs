mod http;
mod log;
mod telegram;

use std::sync::Arc;
use std::time::Duration;

use async_executor::Executor;
use telegram::{Telegram, Update};

fn main() {
    dotenvy::dotenv().ok();

    log::init();
    log::info("Бот запускається...");

    let token = std::env::var("TELEGRAM_TOKEN").expect("Встанови TELEGRAM_TOKEN у файлі .env");
    let bot = Arc::new(Telegram::new(token));

    let ex: Executor<'static> = Executor::new();

    async_io::block_on(ex.run(async {
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
    let Some(message) = update.message else {
        return;
    };
    let Some(text) = message.text else {
        return;
    };
    let chat_id = message.chat.id;

    log::info(&format!("Повідомлення \"{text}\" від chat_id={chat_id}"));

    let reply = match text.trim() {
        "/start" => {
            "Привіт! Я простий бот на Rust 🦀\nНапиши /help, щоб побачити команди.".to_string()
        }
        "/help" => "Доступні команди:\n/start — почати роботу з ботом\n/help — показати це повідомлення\n/fact — показати випадковий факт".to_string(),
        "/fact" => "Цікавий факт: Rust вже 9 років поспіль займає перше місце як \"найулюбленіша мова програмування\" у StackOverflow Survey.".to_string(),
        _ => return,
    };

    if let Err(e) = bot.send_message(chat_id, &reply).await {
        log::error(&format!("Не вдалося відправити повідомлення: {e}"));
    }
}
