from pathlib import Path

import pytest
from pydantic import ValidationError

from mcp_email.config import Settings


def test_settings_normalize_allowlist():
    settings = Settings.model_validate(
        {
            "smtp_host": "smtp.example.org",
            "smtp_port": 587,
            "smtp_username": "mailer",
            "smtp_password": "secret",
            "sender_email": "robot@example.org",
            "allowed_recipients": [" USER@example.org "],
        }
    )

    assert settings.allowed_recipients == ["user@example.org"]
    assert settings.daily_limit == 10
    assert settings.throttle_seconds == 60


def test_settings_accept_comma_separated_allowlist_from_environment(monkeypatch):
    monkeypatch.setenv("SMTP_HOST", "smtp.example.org")
    monkeypatch.setenv("SMTP_PORT", "587")
    monkeypatch.setenv("SMTP_USERNAME", "mailer")
    monkeypatch.setenv("SMTP_PASSWORD", "secret")
    monkeypatch.setenv("SENDER_EMAIL", "robot@example.org")
    monkeypatch.setenv("ALLOWED_RECIPIENTS", " USER@example.org , second@example.org ")

    settings = Settings()

    assert settings.allowed_recipients == ["user@example.org", "second@example.org"]


@pytest.mark.parametrize(
    ("field_name", "value"),
    [
        ("smtp_port", 0),
        ("smtp_port", 70000),
        ("daily_limit", 0),
        ("throttle_seconds", 0),
        ("subject_max_length", 0),
        ("body_max_length", 0),
    ],
)
def test_settings_reject_nonsensical_numeric_limits(field_name, value):
    payload = {
        "smtp_host": "smtp.example.org",
        "smtp_port": 587,
        "smtp_username": "mailer",
        "smtp_password": "secret",
        "sender_email": "robot@example.org",
        "allowed_recipients": ["user@example.org"],
    }
    payload[field_name] = value

    with pytest.raises(ValidationError):
        Settings.model_validate(payload)


@pytest.mark.parametrize("field_name", ["smtp_host", "smtp_username", "smtp_password"])
def test_settings_reject_blank_connection_fields(field_name):
    payload = {
        "smtp_host": "smtp.example.org",
        "smtp_port": 587,
        "smtp_username": "mailer",
        "smtp_password": "secret",
        "sender_email": "robot@example.org",
        "allowed_recipients": ["user@example.org"],
    }
    payload[field_name] = "   "

    with pytest.raises(ValidationError):
        Settings.model_validate(payload)


def test_settings_reject_invalid_quota_timezone():
    with pytest.raises(ValidationError):
        Settings.model_validate(
            {
                "smtp_host": "smtp.example.org",
                "smtp_port": 587,
                "smtp_username": "mailer",
                "smtp_password": "secret",
                "sender_email": "robot@example.org",
                "allowed_recipients": ["user@example.org"],
                "quota_timezone": "Mars/Olympus",
            }
        )


def test_settings_reject_conflicting_ssl_and_starttls():
    with pytest.raises(ValidationError):
        Settings.model_validate(
            {
                "smtp_host": "smtp.example.org",
                "smtp_port": 465,
                "smtp_username": "mailer",
                "smtp_password": "secret",
                "smtp_use_ssl": True,
                "smtp_use_starttls": True,
                "sender_email": "robot@example.org",
                "allowed_recipients": ["user@example.org"],
            }
        )


def test_settings_require_encrypted_smtp_transport():
    with pytest.raises(ValidationError):
        Settings.model_validate(
            {
                "smtp_host": "smtp.example.org",
                "smtp_port": 25,
                "smtp_username": "mailer",
                "smtp_password": "secret",
                "smtp_use_ssl": False,
                "smtp_use_starttls": False,
                "sender_email": "robot@example.org",
                "allowed_recipients": ["user@example.org"],
            }
        )


def test_settings_load_explicit_project_root_outside_repo_root(monkeypatch, tmp_path):
    project_root = tmp_path / "project"
    project_root.mkdir()
    env_path = project_root / ".env"
    monkeypatch.chdir(tmp_path)
    for key in ["SMTP_HOST", "SMTP_PORT", "SMTP_USERNAME", "SMTP_PASSWORD", "SENDER_EMAIL", "ALLOWED_RECIPIENTS", "STORE_PATH", "MCP_EMAIL_PROJECT_ROOT"]:
        monkeypatch.delenv(key, raising=False)

    env_path.write_text(
        "\n".join(
            [
                "SMTP_HOST=smtp.example.org",
                "SMTP_PORT=587",
                "SMTP_USERNAME=mailer",
                "SMTP_PASSWORD=secret",
                "SENDER_EMAIL=robot@example.org",
                "ALLOWED_RECIPIENTS=user@example.org",
                "STORE_PATH=./data/email.sqlite3",
            ]
        )
        + "\n"
    )

    settings = Settings(_project_root=project_root)

    assert settings.smtp_host == "smtp.example.org"
    assert settings.store_path == str(project_root / "data" / "email.sqlite3")


def test_settings_resolve_relative_store_path_from_explicit_env_file(monkeypatch, tmp_path):
    project_root = tmp_path / "custom-root"
    project_root.mkdir()
    env_path = project_root / "custom.env"

    monkeypatch.chdir(tmp_path)
    for key in ["SMTP_HOST", "SMTP_PORT", "SMTP_USERNAME", "SMTP_PASSWORD", "SENDER_EMAIL", "ALLOWED_RECIPIENTS", "STORE_PATH", "MCP_EMAIL_PROJECT_ROOT"]:
        monkeypatch.delenv(key, raising=False)

    env_path.write_text(
        "\n".join(
            [
                "SMTP_HOST=smtp.example.org",
                "SMTP_PORT=587",
                "SMTP_USERNAME=mailer",
                "SMTP_PASSWORD=secret",
                "SENDER_EMAIL=robot@example.org",
                "ALLOWED_RECIPIENTS=user@example.org",
                "STORE_PATH=./data/email.sqlite3",
            ]
        )
        + "\n"
    )

    settings = Settings(_env_file=env_path)

    assert settings.store_path == str(project_root / "data" / "email.sqlite3")
