use reqwest::Client;

#[derive(Clone)]
pub enum EmailService {
    Disabled,
    Resend(ResendEmailSender),
}

#[derive(Clone)]
pub struct ResendEmailSender {
    api_key: String,
    from: String,
    client: Client,
}

impl EmailService {
    pub fn new(resend_api_key: &str, email_from: &str, client: Client) -> Self {
        if resend_api_key.is_empty() || email_from.is_empty() {
            Self::Disabled
        } else {
            Self::Resend(ResendEmailSender {
                api_key: resend_api_key.to_string(),
                from: email_from.to_string(),
                client,
            })
        }
    }

    pub fn is_configured(&self) -> bool {
        matches!(self, Self::Resend(_))
    }

    pub async fn send_password_reset(
        &self,
        to: &str,
        reset_url: &str,
    ) -> Result<(), anyhow::Error> {
        match self {
            Self::Disabled => Ok(()),
            Self::Resend(sender) => sender.send_password_reset(to, reset_url).await,
        }
    }
}

impl ResendEmailSender {
    async fn send_password_reset(
        &self,
        to: &str,
        reset_url: &str,
    ) -> Result<(), anyhow::Error> {
        let html = format!(
            "<p>Здравствуйте!</p>\
             <p>Вы запросили восстановление пароля. Перейдите по ссылке:</p>\
             <p><a href=\"{}\">Восстановить пароль</a></p>\
             <p>Ссылка действительна в течение 1 часа.</p>",
            reset_url
        );

        let response = self
            .client
            .post("https://api.resend.com/emails")
            .bearer_auth(&self.api_key)
            .json(&serde_json::json!({
                "from": self.from,
                "to": [to],
                "subject": "Восстановление пароля",
                "html": html,
            }))
            .send()
            .await?;

        if !response.status().is_success() {
            let body = response.text().await.unwrap_or_default();
            eprintln!("Resend API error: {}", body);
        }

        Ok(())
    }
}
