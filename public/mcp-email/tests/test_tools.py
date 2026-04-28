import sqlite3
from datetime import UTC, datetime

import pytest
from pydantic import ValidationError

from mcp_email.config import Settings
from mcp_email.models import EmailStatus, EmailSubmitRequest
from mcp_email.policy import build_dedupe_fingerprint
from mcp_email.server import create_mcp
from mcp_email.store import SQLiteEmailStore
from mcp_email.tools.email import EmailToolService


def build_settings(approval_required: bool, store_path=None) -> Settings:
    payload = {
        "smtp_host": "smtp.example.org",
        "smtp_port": 587,
        "smtp_username": "mailer",
        "smtp_password": "secret",
        "sender_email": "robot@example.org",
        "allowed_recipients": ["user@example.org"],
        "approval_required": approval_required,
    }
    if store_path is not None:
        payload["store_path"] = str(store_path)
    return Settings.model_validate(payload)


def test_submit_request_defaults_to_pending_approval(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    service = EmailToolService(
        settings=build_settings(True),
        store=store,
        now_fn=lambda: datetime.now(UTC),
    )

    result = service.submit("user@example.org", "Hello", "Body")

    assert result.status == EmailStatus.PENDING_APPROVAL


def test_submit_request_can_be_ready_to_send_when_approval_disabled(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    service = EmailToolService(
        settings=build_settings(False),
        store=store,
        now_fn=lambda: datetime.now(UTC),
    )

    result = service.submit("user@example.org", "Hello", "Body")

    assert result.status == EmailStatus.READY_TO_SEND


def test_service_status_and_recent_history(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    service = EmailToolService(
        settings=build_settings(False),
        store=store,
        now_fn=lambda: datetime.now(UTC),
    )

    created = service.submit("user@example.org", "Hello", "Body")

    assert service.status(str(created.id)).id == created.id
    assert [item.id for item in service.list_recent(limit=5)] == [created.id]


def test_service_rejects_unbounded_recent_history_limit(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    service = EmailToolService(
        settings=build_settings(False),
        store=store,
        now_fn=lambda: datetime.now(UTC),
    )

    with pytest.raises(ValueError, match="limit"):
        service.list_recent(limit=500)


def test_submit_returns_existing_request_for_duplicate_payload(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    now = datetime.now(UTC)
    service = EmailToolService(
        settings=build_settings(False),
        store=store,
        now_fn=lambda: now,
    )

    first = service.submit("user@example.org", "Hello", "Body")
    second = service.submit("user@example.org", "Hello", "Body")

    assert second.id == first.id
    assert [item.id for item in store.list_recent(limit=5)] == [first.id]


def test_submit_uses_existing_reservation_for_duplicate_fingerprint(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    now = datetime.now(UTC)
    settings = build_settings(False)
    service = EmailToolService(
        settings=settings,
        store=store,
        now_fn=lambda: now,
    )
    request = EmailSubmitRequest(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
    )
    fingerprint = build_dedupe_fingerprint(
        request,
        now=now,
        window_seconds=settings.throttle_seconds,
    )

    store.record_reservation(
        quota_day=now.date().isoformat(),
        reserved_at=now,
        dedupe_fingerprint=fingerprint,
    )

    created = service.submit("user@example.org", "Hello", "Body")

    assert created.status == EmailStatus.READY_TO_SEND
    assert store.count_reserved_for_day(now.date().isoformat()) == 1


def test_submit_keeps_idempotency_key_separate_from_dedupe_fingerprint(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    now = datetime.now(UTC)
    settings = build_settings(False)
    service = EmailToolService(
        settings=settings,
        store=store,
        now_fn=lambda: now,
    )
    request = EmailSubmitRequest(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
        request_idempotency_key="retry-1",
    )

    created = service.submit(
        "user@example.org",
        "Hello",
        "Body",
        request_idempotency_key="retry-1",
    )

    assert created.idempotency_key == "retry-1"
    assert created.dedupe_fingerprint == build_dedupe_fingerprint(
        request,
        now=now,
        window_seconds=settings.throttle_seconds,
    )


def test_submit_recovers_from_duplicate_create_race(tmp_path, monkeypatch):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    service = EmailToolService(
        settings=build_settings(False),
        store=store,
        now_fn=lambda: datetime.now(UTC),
    )
    existing = service.submit("user@example.org", "Hello", "Body")
    duplicate = existing.model_copy(update={"id": existing.id})
    calls = {"find": 0}

    def fake_find_by_fingerprint(fingerprint: str):
        calls["find"] += 1
        return None if calls["find"] == 1 else duplicate

    def fake_create_or_get_request(_request):
        raise sqlite3.IntegrityError("duplicate fingerprint")

    monkeypatch.setattr(service.store, "find_by_fingerprint", fake_find_by_fingerprint)
    monkeypatch.setattr(service.store, "create_or_get_request", fake_create_or_get_request)

    returned = service.submit("user@example.org", "Hello", "Body")

    assert returned.id == existing.id


def test_create_mcp_requires_valid_settings(monkeypatch):
    for key in [
        "SMTP_HOST",
        "SMTP_PORT",
        "SMTP_USERNAME",
        "SMTP_PASSWORD",
        "SENDER_EMAIL",
        "ALLOWED_RECIPIENTS",
        "STORE_PATH",
        "MCP_EMAIL_PROJECT_ROOT",
    ]:
        monkeypatch.delenv(key, raising=False)

    with pytest.raises(ValidationError):
        create_mcp()
