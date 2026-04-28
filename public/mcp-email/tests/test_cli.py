from datetime import UTC, datetime

import sqlite3

from mcp_email.cli import main
from mcp_email.models import EmailStatus, StoredEmailRequest
from mcp_email.store import SQLiteEmailStore


def _make_request(*, status: EmailStatus, created_at: datetime) -> StoredEmailRequest:
    return StoredEmailRequest(
        to="user@example.org",
        subject="Hello",
        body_text="Body",
        dedupe_fingerprint=f"fp-{created_at.timestamp()}-{status.value}",
        status=status,
        created_at=created_at,
        updated_at=created_at,
    )


def test_admin_cli_approve_moves_pending_request_to_ready_to_send_and_records_metadata(
    tmp_path, capsys
):
    store_path = tmp_path / "email.sqlite3"
    store = SQLiteEmailStore(store_path)
    store.initialize()
    created_at = datetime.now(UTC)
    request = _make_request(
        status=EmailStatus.PENDING_APPROVAL,
        created_at=created_at,
    )
    store.create_request(request)

    exit_code = main(
        [
            "--store-path",
            str(store_path),
            "approve",
            str(request.id),
            "--actor",
            "alice",
            "--reason",
            "looks good",
        ]
    )

    updated = store.get_request(str(request.id))
    output = capsys.readouterr().out

    assert exit_code == 0
    assert updated is not None
    assert updated.status == EmailStatus.READY_TO_SEND
    assert updated.approval_actor == "alice"
    assert updated.approval_reason == "looks good"
    assert "approved" in output
    assert str(request.id) in output


def test_store_approve_request_moves_pending_request_to_ready_with_metadata(tmp_path):
    store_path = tmp_path / "email.sqlite3"
    store = SQLiteEmailStore(store_path)
    store.initialize()
    created_at = datetime.now(UTC)
    request = _make_request(
        status=EmailStatus.PENDING_APPROVAL,
        created_at=created_at,
    )
    store.create_request(request)

    updated = store.approve_request(
        str(request.id),
        approval_actor="alice",
        approval_reason="looks good",
    )

    assert updated.status == EmailStatus.READY_TO_SEND
    assert updated.approval_actor == "alice"
    assert updated.approval_reason == "looks good"
    assert store.get_request(str(request.id)).status == EmailStatus.READY_TO_SEND


def test_store_reject_request_marks_pending_request_rejected_with_metadata(tmp_path):
    store_path = tmp_path / "email.sqlite3"
    store = SQLiteEmailStore(store_path)
    store.initialize()
    created_at = datetime.now(UTC)
    request = _make_request(
        status=EmailStatus.PENDING_APPROVAL,
        created_at=created_at,
    )
    store.create_request(request)

    updated = store.reject_request(
        str(request.id),
        approval_actor="alice",
        approval_reason="not needed",
    )

    assert updated.status == EmailStatus.REJECTED
    assert updated.approval_actor == "alice"
    assert updated.approval_reason == "not needed"
    assert store.get_request(str(request.id)).status == EmailStatus.REJECTED


def test_admin_cli_reject_records_actor_and_reason(tmp_path, capsys):
    store_path = tmp_path / "email.sqlite3"
    store = SQLiteEmailStore(store_path)
    store.initialize()
    created_at = datetime.now(UTC)
    request = _make_request(
        status=EmailStatus.PENDING_APPROVAL,
        created_at=created_at,
    )
    store.create_request(request)

    exit_code = main(
        [
            "--store-path",
            str(store_path),
            "reject",
            str(request.id),
            "--actor",
            "alice",
            "--reason",
            "not needed",
        ]
    )

    updated = store.get_request(str(request.id))
    output = capsys.readouterr().out

    assert exit_code == 0
    assert updated is not None
    assert updated.status == EmailStatus.REJECTED
    assert updated.approval_actor == "alice"
    assert updated.approval_reason == "not needed"
    assert "rejected" in output
    assert str(request.id) in output


def test_admin_cli_list_pending_shows_only_pending_requests_in_creation_order(tmp_path, capsys):
    store_path = tmp_path / "email.sqlite3"
    store = SQLiteEmailStore(store_path)
    store.initialize()
    first_pending = _make_request(
        status=EmailStatus.PENDING_APPROVAL,
        created_at=datetime(2026, 4, 2, 10, 0, tzinfo=UTC),
    )
    ready = _make_request(
        status=EmailStatus.READY_TO_SEND,
        created_at=datetime(2026, 4, 2, 10, 5, tzinfo=UTC),
    )
    second_pending = _make_request(
        status=EmailStatus.PENDING_APPROVAL,
        created_at=datetime(2026, 4, 2, 10, 10, tzinfo=UTC),
    )
    store.create_request(first_pending)
    store.create_request(ready)
    store.create_request(second_pending)

    exit_code = main(["--store-path", str(store_path), "list-pending"])

    output = capsys.readouterr().out.strip().splitlines()

    assert exit_code == 0
    assert output[0].startswith(str(first_pending.id))
    assert output[1].startswith(str(second_pending.id))
    assert all(str(ready.id) not in line for line in output)


def test_admin_cli_reports_store_initialization_errors(monkeypatch, capsys):
    def fake_load_store(_store_path):
        raise OSError("disk full")

    monkeypatch.setattr("mcp_email.cli._load_store", fake_load_store)

    exit_code = main(["list-pending"])
    error_output = capsys.readouterr().err

    assert exit_code == 1
    assert "disk full" in error_output


def test_admin_cli_reports_command_database_errors(tmp_path, monkeypatch, capsys):
    store_path = tmp_path / "email.sqlite3"
    store = SQLiteEmailStore(store_path)
    store.initialize()

    def fake_list_pending(_store):
        raise sqlite3.OperationalError("db locked")

    monkeypatch.setattr("mcp_email.cli._list_pending", fake_list_pending)

    exit_code = main(["--store-path", str(store_path), "list-pending"])
    error_output = capsys.readouterr().err

    assert exit_code == 1
    assert "db locked" in error_output
    assert "initialize" not in error_output
