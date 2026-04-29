import smtplib, os, logging
from email.mime.text import MIMEText
from email.mime.multipart import MIMEMultipart
from .base import EmailProvider

logger = logging.getLogger(__name__)

class MailerSendProvider(EmailProvider):
    def __init__(self):
        self.host = "smtp.mailersend.net"
        self.port = 465
        self.username = os.getenv("MAILERSEND_SMTP_USER")
        self.password = os.getenv("MAILERSEND_SMTP_PASS")
        self.from_email = os.getenv("EMAIL_FROM", "no-reply@yourdomain.com")

    def send(self, to: str, subject: str, html: str) -> bool:
        try:
            msg = MIMEMultipart("alternative")
            msg["Subject"] = subject
            msg["From"] = self.from_email
            msg["To"] = to
            msg.attach(MIMEText(html, "html"))

            with smtplib.SMTP_SSL(self.host, self.port) as server:  # ← SMTP_SSL вместо SMTP + starttls
                server.login(self.username, self.password)
                server.sendmail(self.from_email, to, msg.as_string())
            return True
        except Exception as e:
            logger.error(f"MailerSend SMTP failed: {e}")
            return False
