from mcp_email.config import Settings
from mcp_email.server import create_mcp


def build_settings(tmp_path):
    return Settings.model_validate(
        {
            "smtp_host": "smtp.example.org",
            "smtp_port": 587,
            "smtp_username": "mailer",
            "smtp_password": "secret",
            "sender_email": "robot@example.org",
            "allowed_recipients": ["user@example.org"],
            "store_path": str(tmp_path / "email.sqlite3"),
        }
    )


def test_create_mcp_returns_server_like_object(tmp_path):
    app = create_mcp(settings=build_settings(tmp_path))
    assert hasattr(app, "run")


def test_create_mcp_registers_email_tools(monkeypatch, tmp_path):
    seen = {}

    def fake_register_email_tools(mcp, *, settings, store):
        seen["mcp"] = mcp
        seen["settings"] = settings
        seen["store_path"] = store.path

    monkeypatch.setattr("mcp_email.server.register_email_tools", fake_register_email_tools)

    settings = build_settings(tmp_path)
    app = create_mcp(settings=settings)

    assert seen["mcp"] is app
    assert seen["settings"] == settings
    assert seen["store_path"] == tmp_path / "email.sqlite3"
