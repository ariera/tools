from datetime import UTC, datetime

import pytest

from mcp_email.config import Settings
from mcp_email.models import EmailSubmitRequest
from mcp_email.policy import (
    PolicyViolation,
    build_dedupe_fingerprint,
    validate_submit_request,
)
from mcp_email.rate_limits import QuotaExceeded, RateLimiter
from mcp_email.store import SQLiteEmailStore


def build_settings() -> Settings:
    return Settings.model_validate(
        {
            "smtp_host": "smtp.example.org",
            "smtp_port": 587,
            "smtp_username": "mailer",
            "smtp_password": "secret",
            "sender_email": "robot@example.org",
            "allowed_recipients": ["user@example.org"],
        }
    )


def test_validate_submit_request_rejects_non_allowlisted_recipient():
    request = EmailSubmitRequest(
        to="other@example.org",
        subject="Hello",
        body_text="Body",
    )

    with pytest.raises(PolicyViolation):
        validate_submit_request(request, build_settings())


def test_validate_submit_request_rejects_header_injection():
    request = EmailSubmitRequest.model_construct(
        to="user@example.org",
        subject="Hello\nBcc: bad@example.org",
        body_text="Body",
        reason=None,
        request_idempotency_key=None,
    )

    with pytest.raises(PolicyViolation):
        validate_submit_request(request, build_settings())


def test_validate_submit_request_rejects_recipient_header_injection():
    request = EmailSubmitRequest.model_construct(
        to="user@example.org\r",
        subject="Hello",
        body_text="Body",
        reason=None,
        request_idempotency_key=None,
    )

    with pytest.raises(PolicyViolation):
        validate_submit_request(request, build_settings())


def test_build_dedupe_fingerprint_is_stable():
    request = EmailSubmitRequest(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
    )

    assert build_dedupe_fingerprint(request) == build_dedupe_fingerprint(request)


def test_rate_limiter_enforces_daily_limit(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    limiter = RateLimiter(store=store, daily_limit=1, throttle_seconds=60, timezone_name="UTC")
    now = datetime.now(UTC)

    limiter.reserve_slot("user@example.org", "fp-1", now=now)

    with pytest.raises(QuotaExceeded):
        limiter.reserve_slot("user@example.org", "fp-2", now=now)

    assert store.count_reserved_for_day(now.date().isoformat()) == 1
