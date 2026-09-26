use reqwest::Client;

#[derive(Clone)]
pub struct TelegramNotifier {
    bot_token: String,
    client: Client,
    base_url: String,
}

impl TelegramNotifier {
    pub fn new(bot_token: String) -> Self {
        Self::with_base_url(bot_token, "https://api.telegram.org".to_string())
    }

    /// Build a notifier against a custom API base URL.
    ///
    /// Production always uses [`Self::new`]; this exists so tests can point the
    /// notifier at a local stub instead of the real Telegram API.
    pub fn with_base_url(bot_token: String, base_url: String) -> Self {
        Self {
            bot_token,
            client: crate::services::external::http_client(),
            base_url,
        }
    }

    pub fn is_configured(&self) -> bool {
        !self.bot_token.is_empty()
    }

    pub async fn send_message(&self, chat_id: &str, text: &str) -> Result<(), anyhow::Error> {
        if !self.is_configured() {
            return Ok(());
        }

        let url = format!("{}/bot{}/sendMessage", self.base_url, self.bot_token);

        let response = self
            .client
            .post(&url)
            .json(&serde_json::json!({
                "chat_id": chat_id,
                "text": text,
                "parse_mode": "HTML",
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("Telegram API error {}: {}", status, body);
        }

        Ok(())
    }

    pub async fn send_new_episode_notification(
        &self,
        chat_id: &str,
        title: &str,
        episode: i32,
    ) -> Result<(), anyhow::Error> {
        let text = format!("🎬 <b>Новая серия!</b>\n\n{} — серия {}", title, episode);
        self.send_message(chat_id, &text).await
    }

    pub async fn send_test_message(&self, chat_id: &str) -> Result<(), anyhow::Error> {
        self.send_message(chat_id, "✅ Telegram-уведомления настроены!")
            .await
    }
}
