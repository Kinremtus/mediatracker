import resend, os, logging
from .base import EmailProvider

logger = logging.getLogger(__name__)

load_dotenv(os.path.join(os.path.dirname(file), "../.env"))

class ResendProvider(EmailProvider):
    def __init__(self):
        resend.api_key = os.getenv("RESEND_API_KEY")
        self.from_email = os.getenv("EMAIL_FROM", "no-reply@yourdomain.com")

    def send(self, to: str, subject: str, html: str) -> bool:
        try:
            resend.Emails.send({
                "from": self.from_email,
                "to": to,
                "subject": subject,
                "html": html
            })
            return True
        except Exception as e:
            logger.error(f"Resend failed: {e}")
            return False