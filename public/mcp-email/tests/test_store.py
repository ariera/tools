import os
import stat
from datetime import UTC, datetime

import pytest

from mcp_email.models import EmailStatus, StoredEmailRequest
from mcp_email.store import SQLiteEmailStore


def test_store_creates_and_reads_request(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()

    request = StoredEmailRequest(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
        dedupe_fingerprint="fp-1",
        status=EmailStatus.PENDING_APPROVAL,
        created_at=datetime.now(UTC),
        updated_at=datetime.now(UTC),
    )

    store.create_request(request)
    loaded = store.get_request(str(request.id))

    assert loaded is not None
    assert loaded.subject == "Hello"
    assert loaded.status == EmailStatus.PENDING_APPROVAL


def test_store_lists_recent_requests(tmp_path):
    store = SQLiteEmailStore(tmp_path / "email.sqlite3")
    store.initialize()

    assert store.list_recent(limit=10) == []


def test_store_claims_ready_request_once(tmp_path):
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

    claimed = store.claim_next_ready_request()

    assert claimed is not None
    assert claimed.id == request.id
    assert claimed.status == EmailStatus.SENDING
    assert store.claim_next_ready_request() is None


def test_store_initialize_sets_owner_only_permissions(tmp_path):
    store_path = tmp_path / "nested" / "email.sqlite3"
    store = SQLiteEmailStore(store_path)

    store.initialize()

    directory_mode = stat.S_IMODE(os.stat(store_path.parent).st_mode)
    file_mode = stat.S_IMODE(os.stat(store_path).st_mode)

    assert directory_mode == 0o700
    assert file_mode == 0o600


def test_store_initialize_rejects_symlink_path(tmp_path):
    target = tmp_path / "target.sqlite3"
    target.write_text("existing")
    symlink_path = tmp_path / "email.sqlite3"
    symlink_path.symlink_to(target)
    store = SQLiteEmailStore(symlink_path)

    with pytest.raises(ValueError, match="symlink"):
        store.initialize()
