from datetime import UTC, datetime

import pytest

from mcp_email.dispatcher import EmailDispatcher
from mcp_email.models import EmailStatus, StoredEmailRequest
from mcp_email.store import SQLiteEmailStore


class FakeTransport:
    def __init__(self):
        self.calls = []

    def send_plain_text(self, *, to: str, subject: str, body_text: str) -> str:
        self.calls.append((to, subject, body_text))
        return "message-id-1"


class FailingTransport:
    def send_plain_text(self, *, to: str, subject: str, body_text: str) -> str:
        raise RuntimeError("smtp refused recipient")


def test_dispatcher_sends_ready_request(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    request = StoredEmailRequest(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
        dedupe_fingerprint="fp-1",
        status=EmailStatus.READY_TO_SEND,
        created_at=datetime.now(UTC),
        updated_at=datetime.now(UTC),
    )
    store.create_request(request)
    dispatcher = EmailDispatcher(store=store, transport=FakeTransport())

    dispatcher.dispatch_once()

    loaded = store.get_request(str(request.id))
    assert loaded.status == EmailStatus.SENT
    assert loaded.transport_message_id == "message-id-1"


def test_dispatcher_marks_request_failed_when_transport_raises(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()
    request = StoredEmailRequest(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
        dedupe_fingerprint="fp-1",
        status=EmailStatus.READY_TO_SEND,
        created_at=datetime.now(UTC),
        updated_at=datetime.now(UTC),
    )
    store.create_request(request)
    dispatcher = EmailDispatcher(store=store, transport=FailingTransport())

    with pytest.raises(RuntimeError, match="smtp refused recipient"):
        dispatcher.dispatch_once()

    loaded = store.get_request(str(request.id))
    assert loaded.status == EmailStatus.FAILED
    assert loaded.error_message == "smtp refused recipient"
