import pytest
from pydantic import ValidationError

from mcp_email.models import EmailStatus, EmailSubmitRequest, StoredEmailRequest


def test_submit_request_forbids_unknown_fields():
    with pytest.raises(ValidationError):
        EmailSubmitRequest.model_validate(
            {
                "to": "user@example.org",
                "subject": "Hello",
                "body_text": "Body",
                "attachments": ["secret.pdf"],
            }
        )


@pytest.mark.parametrize(
    ("field_name", "value"),
    [
        ("subject", "   "),
        ("body_text", "\n\t "),
        ("subject", "Hello\r\nBcc: victim@example.org"),
    ],
)
def test_submit_request_rejects_blank_required_text_fields(field_name, value):
    payload = {
        "to": "user@example.org",
        "subject": "Hello",
        "body_text": "Body",
    }
    payload[field_name] = value

    with pytest.raises(ValidationError):
        EmailSubmitRequest.model_validate(payload)


def test_status_enum_contains_pending_approval():
    assert EmailStatus.PENDING_APPROVAL.value == "pending_approval"


def test_submit_request_preserves_body_text_whitespace_and_normalizes_optional_text():
    request = EmailSubmitRequest.model_validate(
        {
            "to": "user@example.org",
            "subject": "  Hello  ",
            "body_text": "\n  Body with spaces  \n",
            "reason": "   ",
            "request_idempotency_key": "  key-123  ",
        }
    )

    assert request.subject == "Hello"
    assert request.body_text == "\n  Body with spaces  \n"
    assert request.reason is None
    assert request.request_idempotency_key == "key-123"


def test_stored_email_request_defaults_id_and_keeps_status_and_datetimes():
    request = StoredEmailRequest.model_validate(
        {
            "to": "user@example.org",
            "subject": "Hello",
            "body_text": "Body",
            "dedupe_fingerprint": "abc123",
            "status": "pending_approval",
            "created_at": "2026-04-02T10:00:00Z",
            "updated_at": "2026-04-02T10:01:00Z",
        }
    )

    assert request.id is not None
    assert request.status is EmailStatus.PENDING_APPROVAL
    assert request.created_at.isoformat() == "2026-04-02T10:00:00+00:00"
    assert request.updated_at.isoformat() == "2026-04-02T10:01:00+00:00"


def test_stored_email_request_preserves_body_text_and_normalizes_text_fields():
    request = StoredEmailRequest.model_validate(
        {
            "to": "user@example.org",
            "subject": "  Hello  ",
            "body_text": "\n  Body with spaces  \n",
            "reason": "   ",
            "idempotency_key": "  abc123  ",
            "dedupe_fingerprint": "  fp-1  ",
            "status": "pending_approval",
            "created_at": "2026-04-02T10:00:00Z",
            "updated_at": "2026-04-02T10:01:00Z",
        }
    )

    assert request.subject == "Hello"
    assert request.body_text == "\n  Body with spaces  \n"
    assert request.reason is None
    assert request.idempotency_key == "abc123"
    assert request.dedupe_fingerprint == "fp-1"


def test_stored_email_request_forbids_unknown_fields():
    with pytest.raises(ValidationError):
        StoredEmailRequest.model_validate(
            {
                "to": "user@example.org",
                "subject": "Hello",
                "body_text": "Body",
                "dedupe_fingerprint": "abc123",
                "status": "pending_approval",
                "created_at": "2026-04-02T10:00:00Z",
                "updated_at": "2026-04-02T10:01:00Z",
                "attachments": ["secret.pdf"],
            }
        )


@pytest.mark.parametrize(
    ("field_name", "value"),
    [
        ("subject", "   "),
        ("body_text", "\n\t "),
        ("subject", "Hello\r\nBcc: victim@example.org"),
    ],
)
def test_stored_email_request_rejects_blank_required_text_fields(field_name, value):
    payload = {
        "to": "user@example.org",
        "subject": "Hello",
        "body_text": "Body",
        "dedupe_fingerprint": "abc123",
        "status": "pending_approval",
        "created_at": "2026-04-02T10:00:00Z",
        "updated_at": "2026-04-02T10:01:00Z",
    }
    payload[field_name] = value

    with pytest.raises(ValidationError):
        StoredEmailRequest.model_validate(payload)


def test_stored_email_request_rejects_blank_dedupe_fingerprint():
    with pytest.raises(ValidationError):
        StoredEmailRequest.model_validate(
            {
                "to": "user@example.org",
                "subject": "Hello",
                "body_text": "Body",
                "dedupe_fingerprint": "   ",
                "status": "pending_approval",
                "created_at": "2026-04-02T10:00:00Z",
                "updated_at": "2026-04-02T10:01:00Z",
            }
        )


@pytest.mark.parametrize("field_name", ["created_at", "updated_at"])
def test_stored_email_request_rejects_naive_datetimes(field_name):
    payload = {
        "to": "user@example.org",
        "subject": "Hello",
        "body_text": "Body",
        "dedupe_fingerprint": "abc123",
        "status": "pending_approval",
        "created_at": "2026-04-02T10:00:00Z",
        "updated_at": "2026-04-02T10:01:00Z",
    }
    payload[field_name] = "2026-04-02T10:00:00"

    with pytest.raises(ValidationError):
        StoredEmailRequest.model_validate(payload)
