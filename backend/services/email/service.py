from .resend_provider import ResendProvider
from .mailersend_provider import MailerSendProvider

class EmailService:
    def __init__(self):
        self.providers = [
            ResendProvider(),       # основной (API)
            MailerSendProvider(),   # резервный (SMTP)
        ]

    def send_welcome(self, to: str, username: str) -> bool:
        subject = "Добро пожаловать в Mediatracker!"
        html = f"<h1>Привет, {username}!</h1><p>Рады видеть тебя в нашем сервисе.</p>"
        
        for provider in self.providers:
            if provider.send(to, subject, html):
                return True
        return False

# Создаем синглтон для использования в приложении и тестах
email_service = EmailService()
