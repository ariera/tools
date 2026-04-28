import ssl

from mcp_email.config import Settings
from mcp_email.smtp_client import SMTPTransport


def build_settings() -> Settings:
    return Settings.model_validate(
        {
            "smtp_host": "smtp.example.org",
            "smtp_port": 587,
            "smtp_username": "mailer",
            "smtp_password": "secret",
            "sender_email": "robot@example.org",
            "allowed_recipients": ["user@example.org"],
            "smtp_timeout_seconds": 15,
        }
    )


def test_smtp_transport_raises_when_server_rejects_recipient(monkeypatch):
    class FakeSMTP:
        def __init__(self, host, port, *, timeout):
            self.host = host
            self.port = port
            self.timeout = timeout

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

        def starttls(self, *, context):
            return None

        def login(self, username, password):
            return None

        def send_message(self, message):
            return {"user@example.org": (550, b"rejected")}

    monkeypatch.setattr("mcp_email.smtp_client.smtplib.SMTP", FakeSMTP)
    transport = SMTPTransport(build_settings())

    try:
        transport.send_plain_text(
            to="user@example.org",
            subject="Hello",
            body_text="Body",
        )
    except RuntimeError as exc:
        assert "user@example.org" in str(exc)
    else:
        raise AssertionError("Expected partial SMTP refusal to raise")


def test_smtp_transport_uses_timeout_and_verified_starttls_context(monkeypatch):
    seen = {}

    class FakeSMTP:
        def __init__(self, host, port, *, timeout):
            seen["host"] = host
            seen["port"] = port
            seen["timeout"] = timeout

        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc, tb):
            return False

        def starttls(self, *, context):
            seen["context"] = context

        def login(self, username, password):
            seen["username"] = username
            seen["password"] = password

        def send_message(self, message):
            seen["subject"] = message["Subject"]
            return {}

    monkeypatch.setattr("mcp_email.smtp_client.smtplib.SMTP", FakeSMTP)
    transport = SMTPTransport(build_settings())

    message_id = transport.send_plain_text(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
    )

    assert message_id == "smtp-accepted"
    assert seen["timeout"] == 15
    assert isinstance(seen["context"], ssl.SSLContext)
    assert seen["subject"] == "Hello"
