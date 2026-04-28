import json
import os
import sqlite3
from datetime import UTC, datetime
from pathlib import Path

from mcp_email.models import EmailStatus, StoredEmailRequest


class SQLiteEmailStore:
    def __init__(self, path: Path | str):
        self.path = Path(path)

    def initialize(self) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        os.chmod(self.path.parent, 0o700)
        if self.path.is_symlink():
            raise ValueError("store path must not be a symlink")
        with sqlite3.connect(self.path) as conn:
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS email_requests (
                    id TEXT PRIMARY KEY,
                    payload_json TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                )
                """
            )
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS quota_reservations (
                    quota_day TEXT NOT NULL,
                    reserved_at TEXT NOT NULL,
                    dedupe_fingerprint TEXT NOT NULL UNIQUE
                )
                """
            )
        os.chmod(self.path, 0o600)

    def create_request(self, request: StoredEmailRequest) -> None:
        with sqlite3.connect(self.path) as conn:
            conn.execute(
                """
                INSERT INTO email_requests (id, payload_json, status, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?)
                """,
                (
                    str(request.id),
                    request.model_dump_json(),
                    request.status.value,
                    request.created_at.isoformat(),
                    request.updated_at.isoformat(),
                ),
            )

    def create_or_get_request(
        self,
        request: StoredEmailRequest,
    ) -> StoredEmailRequest:
        with sqlite3.connect(self.path) as conn:
            conn.execute("BEGIN IMMEDIATE")
            existing = self._find_existing_request(
                conn,
                idempotency_key=request.idempotency_key,
                dedupe_fingerprint=request.dedupe_fingerprint,
            )
            if existing is not None:
                return existing
            conn.execute(
                """
                INSERT INTO email_requests (id, payload_json, status, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?)
                """,
                (
                    str(request.id),
                    request.model_dump_json(),
                    request.status.value,
                    request.created_at.isoformat(),
                    request.updated_at.isoformat(),
                ),
            )
        return request

    def get_request(self, request_id: str) -> StoredEmailRequest | None:
        with sqlite3.connect(self.path) as conn:
            row = conn.execute(
                "SELECT payload_json FROM email_requests WHERE id = ?",
                (request_id,),
            ).fetchone()
        if row is None:
            return None
        return StoredEmailRequest.model_validate(json.loads(row[0]))

    def update_status(
        self,
        request_id: str,
        status: EmailStatus,
        *,
        approval_actor: str | None = None,
        approval_reason: str | None = None,
        transport_message_id: str | None = None,
        error_message: str | None = None,
    ) -> StoredEmailRequest:
        request = self.get_request(request_id)
        if request is None:
            raise KeyError(request_id)
        now = datetime.now(UTC)
        updated = request.model_copy(
            update={
                "status": status,
                "updated_at": now,
                "approval_actor": approval_actor or request.approval_actor,
                "approval_reason": approval_reason or request.approval_reason,
                "transport_message_id": transport_message_id
                or request.transport_message_id,
                "error_message": error_message or request.error_message,
            }
        )
        with sqlite3.connect(self.path) as conn:
            conn.execute(
                """
                UPDATE email_requests
                SET payload_json = ?, status = ?, updated_at = ?
                WHERE id = ?
                """,
                (
                    updated.model_dump_json(),
                    updated.status.value,
                    now.isoformat(),
                    request_id,
                ),
            )
        return updated

    def approve_request(
        self,
        request_id: str,
        *,
        approval_actor: str,
        approval_reason: str | None = None,
    ) -> StoredEmailRequest:
        with sqlite3.connect(self.path) as conn:
            conn.execute("BEGIN IMMEDIATE")
            row = conn.execute(
                "SELECT payload_json FROM email_requests WHERE id = ?",
                (request_id,),
            ).fetchone()
            if row is None:
                raise KeyError(request_id)
            request = StoredEmailRequest.model_validate(json.loads(row[0]))
            if request.status is not EmailStatus.PENDING_APPROVAL:
                raise ValueError(
                    f"request {request_id} is not pending approval (current status: {request.status.value})"
                )
            now = datetime.now(UTC)
            approved = request.model_copy(
                update={
                    "status": EmailStatus.APPROVED,
                    "updated_at": now,
                    "approval_actor": approval_actor,
                    "approval_reason": approval_reason,
                }
            )
            ready = approved.model_copy(
                update={
                    "status": EmailStatus.READY_TO_SEND,
                    "updated_at": now,
                }
            )
            conn.execute(
                """
                UPDATE email_requests
                SET payload_json = ?, status = ?, updated_at = ?
                WHERE id = ?
                """,
                (
                    approved.model_dump_json(),
                    approved.status.value,
                    now.isoformat(),
                    request_id,
                ),
            )
            conn.execute(
                """
                UPDATE email_requests
                SET payload_json = ?, status = ?, updated_at = ?
                WHERE id = ?
                """,
                (
                    ready.model_dump_json(),
                    ready.status.value,
                    now.isoformat(),
                    request_id,
                ),
            )
        return ready

    def reject_request(
        self,
        request_id: str,
        *,
        approval_actor: str,
        approval_reason: str,
    ) -> StoredEmailRequest:
        with sqlite3.connect(self.path) as conn:
            conn.execute("BEGIN IMMEDIATE")
            row = conn.execute(
                "SELECT payload_json FROM email_requests WHERE id = ?",
                (request_id,),
            ).fetchone()
            if row is None:
                raise KeyError(request_id)
            request = StoredEmailRequest.model_validate(json.loads(row[0]))
            if request.status is not EmailStatus.PENDING_APPROVAL:
                raise ValueError(
                    f"request {request_id} is not pending approval (current status: {request.status.value})"
                )
            now = datetime.now(UTC)
            rejected = request.model_copy(
                update={
                    "status": EmailStatus.REJECTED,
                    "updated_at": now,
                    "approval_actor": approval_actor,
                    "approval_reason": approval_reason,
                }
            )
            conn.execute(
                """
                UPDATE email_requests
                SET payload_json = ?, status = ?, updated_at = ?
                WHERE id = ?
                """,
                (
                    rejected.model_dump_json(),
                    rejected.status.value,
                    now.isoformat(),
                    request_id,
                ),
            )
        return rejected

    def find_by_idempotency_key(self, idempotency_key: str) -> StoredEmailRequest | None:
        with sqlite3.connect(self.path) as conn:
            return self._find_by_idempotency_key(conn, idempotency_key)

    def find_by_fingerprint(
        self,
        fingerprint: str,
    ) -> StoredEmailRequest | None:
        with sqlite3.connect(self.path) as conn:
            return self._find_by_fingerprint(conn, fingerprint)

    def list_recent(self, limit: int = 20) -> list[StoredEmailRequest]:
        with sqlite3.connect(self.path) as conn:
            rows = conn.execute(
                """
                SELECT payload_json
                FROM email_requests
                ORDER BY created_at DESC
                LIMIT ?
                """,
                (limit,),
            ).fetchall()
        return [StoredEmailRequest.model_validate(json.loads(row[0])) for row in rows]

    def list_by_status(self, status: EmailStatus) -> list[StoredEmailRequest]:
        with sqlite3.connect(self.path) as conn:
            rows = conn.execute(
                """
                SELECT payload_json
                FROM email_requests
                WHERE status = ?
                ORDER BY created_at ASC
                """,
                (status.value,),
            ).fetchall()
        return [StoredEmailRequest.model_validate(json.loads(row[0])) for row in rows]

    def reserve_quota_slot(
        self,
        *,
        quota_day: str,
        reserved_at: datetime,
        dedupe_fingerprint: str,
        daily_limit: int,
        throttle_seconds: int,
    ) -> str | None:
        with sqlite3.connect(self.path) as conn:
            conn.execute("BEGIN IMMEDIATE")
            row = conn.execute(
                """
                SELECT 1
                FROM quota_reservations
                WHERE dedupe_fingerprint = ?
                """,
                (dedupe_fingerprint,),
            ).fetchone()
            if row is not None:
                return None
            row = conn.execute(
                "SELECT COUNT(*) FROM quota_reservations WHERE quota_day = ?",
                (quota_day,),
            ).fetchone()
            if int(row[0]) >= daily_limit:
                return "Daily email limit exceeded"
            row = conn.execute(
                "SELECT reserved_at FROM quota_reservations ORDER BY reserved_at DESC LIMIT 1"
            ).fetchone()
            last_reserved_at = None if row is None else datetime.fromisoformat(row[0])
            if (
                last_reserved_at is not None
                and (reserved_at - last_reserved_at).total_seconds() < throttle_seconds
            ):
                return "Throttle window active"
            conn.execute(
                """
                INSERT INTO quota_reservations (quota_day, reserved_at, dedupe_fingerprint)
                VALUES (?, ?, ?)
                """,
                (quota_day, reserved_at.isoformat(), dedupe_fingerprint),
            )
        return None

    def claim_next_ready_request(self) -> StoredEmailRequest | None:
        with sqlite3.connect(self.path) as conn:
            conn.execute("BEGIN IMMEDIATE")
            row = conn.execute(
                """
                SELECT id, payload_json
                FROM email_requests
                WHERE status = ?
                ORDER BY created_at ASC
                LIMIT 1
                """,
                (EmailStatus.READY_TO_SEND.value,),
            ).fetchone()
            if row is None:
                return None
            request = StoredEmailRequest.model_validate(json.loads(row[1]))
            now = datetime.now(UTC)
            updated = request.model_copy(
                update={
                    "status": EmailStatus.SENDING,
                    "updated_at": now,
                }
            )
            conn.execute(
                """
                UPDATE email_requests
                SET payload_json = ?, status = ?, updated_at = ?
                WHERE id = ? AND status = ?
                """,
                (
                    updated.model_dump_json(),
                    updated.status.value,
                    now.isoformat(),
                    row[0],
                    EmailStatus.READY_TO_SEND.value,
                ),
            )
        return updated

    def record_reservation(self, *, quota_day: str, reserved_at, dedupe_fingerprint: str) -> None:
        with sqlite3.connect(self.path) as conn:
            conn.execute(
                """
                INSERT INTO quota_reservations (quota_day, reserved_at, dedupe_fingerprint)
                VALUES (?, ?, ?)
                """,
                (quota_day, reserved_at.isoformat(), dedupe_fingerprint),
            )

    def count_reserved_for_day(self, quota_day: str) -> int:
        with sqlite3.connect(self.path) as conn:
            row = conn.execute(
                "SELECT COUNT(*) FROM quota_reservations WHERE quota_day = ?",
                (quota_day,),
            ).fetchone()
        return int(row[0])

    def get_last_reserved_at(self):
        with sqlite3.connect(self.path) as conn:
            row = conn.execute(
                "SELECT reserved_at FROM quota_reservations ORDER BY reserved_at DESC LIMIT 1"
            ).fetchone()
        return None if row is None else datetime.fromisoformat(row[0])

    def _find_existing_request(
        self,
        conn: sqlite3.Connection,
        *,
        idempotency_key: str | None,
        dedupe_fingerprint: str,
    ) -> StoredEmailRequest | None:
        if idempotency_key is not None:
            existing = self._find_by_idempotency_key(conn, idempotency_key)
            if existing is not None:
                return existing
        return self._find_by_fingerprint(conn, dedupe_fingerprint)

    def _find_by_idempotency_key(
        self,
        conn: sqlite3.Connection,
        idempotency_key: str,
    ) -> StoredEmailRequest | None:
        row = conn.execute(
            """
            SELECT payload_json
            FROM email_requests
            WHERE json_extract(payload_json, '$.idempotency_key') = ?
            ORDER BY created_at DESC
            LIMIT 1
            """,
            (idempotency_key,),
        ).fetchone()
        return None if row is None else StoredEmailRequest.model_validate(json.loads(row[0]))

    def _find_by_fingerprint(
        self,
        conn: sqlite3.Connection,
        fingerprint: str,
    ) -> StoredEmailRequest | None:
        query = """
            SELECT payload_json
            FROM email_requests
            WHERE json_extract(payload_json, '$.dedupe_fingerprint') = ?
        """
        params: list[str] = [fingerprint]
        query += " ORDER BY created_at DESC LIMIT 1"
        row = conn.execute(query, params).fetchone()
        return None if row is None else StoredEmailRequest.model_validate(json.loads(row[0]))
