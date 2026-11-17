# Rust-Telegram-Bot

A minimalist Telegram bot written in Rust. No heavy frameworks: a hand-rolled async HTTP client on top of `async-io` + `async-tls`, using long polling via `getUpdates`.

## Commands

- `/start` — greeting
- `/help` — list of commands
- `/fact` — a fun fact about Rust

## Building

1. Create a `.env` file in the project root:

   ```
   TELEGRAM_TOKEN=your_bot_token_here
   ```

2. Build and run:

   ```
   cargo run --release
   ```

Logs are written to `bot.log`, and also to stdout when run in a terminal.

## Dependencies

- `async-executor`, `async-io`, `futures-rustls`, `futures-lite` — async runtime and TLS
- `webpki-roots` — certificate verification
- `time-format` — time zone and time for logging
- `serde` / `serde_json` — Telegram Bot API (de)serialization
- `dotenvy` — loading `.env`

## License

This project is licensed under the MIT License — see [LICENSE](LICENSE).
