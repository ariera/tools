import smtplib
import ssl
from email.message import EmailMessage

from mcp_email.config import Settings


class SMTPTransport:
    def __init__(self, settings: Settings):
        self.settings = settings

    def send_plain_text(self, *, to: str, subject: str, body_text: str) -> str:
        message = EmailMessage()
        message["From"] = self.settings.sender_email
        message["To"] = to
        message["Subject"] = subject
        message.set_content(body_text)
        tls_context = ssl.create_default_context()

        smtp_cls = smtplib.SMTP_SSL if self.settings.smtp_use_ssl else smtplib.SMTP
        smtp_kwargs = {"timeout": self.settings.smtp_timeout_seconds}
        if self.settings.smtp_use_ssl:
            smtp_kwargs["context"] = tls_context
        with smtp_cls(self.settings.smtp_host, self.settings.smtp_port, **smtp_kwargs) as client:
            if self.settings.smtp_use_starttls and not self.settings.smtp_use_ssl:
                client.starttls(context=tls_context)
            client.login(self.settings.smtp_username, self.settings.smtp_password)
            result = client.send_message(message)

        if result:
            rejected = ", ".join(sorted(result))
            raise RuntimeError(f"SMTP rejected recipients: {rejected}")
        return "smtp-accepted"
