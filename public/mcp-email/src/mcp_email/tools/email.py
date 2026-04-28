import sqlite3
from datetime import UTC, datetime

from mcp_email.config import Settings
from mcp_email.models import EmailStatus, EmailSubmitRequest, StoredEmailRequest
from mcp_email.policy import build_dedupe_fingerprint, validate_submit_request
from mcp_email.rate_limits import RateLimiter
from mcp_email.store import SQLiteEmailStore


class EmailToolService:
    MAX_RECENT_REQUESTS = 100

    def __init__(self, *, settings: Settings, store: SQLiteEmailStore, now_fn=None):
        self.settings = settings
        self.store = store
        self.now_fn = now_fn or (lambda: datetime.now(UTC))
        self.rate_limiter = RateLimiter(
            store=store,
            daily_limit=settings.daily_limit,
            throttle_seconds=settings.throttle_seconds,
            timezone_name=settings.quota_timezone,
        )

    def submit(
        self,
        to: str,
        subject: str,
        body_text: str,
        reason: str | None = None,
        request_idempotency_key: str | None = None,
    ) -> StoredEmailRequest:
        request = validate_submit_request(
            EmailSubmitRequest(
                to=to,
                subject=subject,
                body_text=body_text,
                reason=reason,
                request_idempotency_key=request_idempotency_key,
            ),
            self.settings,
        )
        now = self.now_fn()
        fingerprint = build_dedupe_fingerprint(
            request,
            now=now,
            window_seconds=self.settings.throttle_seconds,
        )
        if request.request_idempotency_key is not None:
            existing = self.store.find_by_idempotency_key(request.request_idempotency_key)
            if existing is not None:
                return existing
        existing = self.store.find_by_fingerprint(fingerprint)
        if existing is not None:
            return existing

        self.rate_limiter.reserve_slot(request.to, fingerprint, now=now)
        stored = StoredEmailRequest(
            to=request.to,
            subject=request.subject,
            body_text=request.body_text,
            reason=request.reason,
            idempotency_key=request.request_idempotency_key,
            dedupe_fingerprint=fingerprint,
            status=(
                EmailStatus.PENDING_APPROVAL
                if self.settings.approval_required
                else EmailStatus.READY_TO_SEND
            ),
            created_at=now,
            updated_at=now,
        )
        try:
            return self.store.create_or_get_request(stored)
        except sqlite3.IntegrityError:
            if request.request_idempotency_key is not None:
                existing = self.store.find_by_idempotency_key(request.request_idempotency_key)
                if existing is not None:
                    return existing
            existing = self.store.find_by_fingerprint(fingerprint)
            if existing is not None:
                return existing
            raise

    def status(self, request_id: str) -> StoredEmailRequest:
        request = self.store.get_request(request_id)
        if request is None:
            raise KeyError(request_id)
        return request

    def list_recent(self, limit: int = 20) -> list[StoredEmailRequest]:
        if limit < 1 or limit > self.MAX_RECENT_REQUESTS:
            raise ValueError(f"limit must be between 1 and {self.MAX_RECENT_REQUESTS}")
        return self.store.list_recent(limit=limit)


def register_email_tools(mcp, *, settings: Settings, store: SQLiteEmailStore) -> None:
    service = EmailToolService(settings=settings, store=store)

    @mcp.tool(name="email_submit")
    def email_submit(
        to: str,
        subject: str,
        body_text: str,
        reason: str | None = None,
        request_idempotency_key: str | None = None,
    ) -> dict:
        created = service.submit(
            to,
            subject,
            body_text,
            reason=reason,
            request_idempotency_key=request_idempotency_key,
        )
        return {"id": str(created.id), "status": created.status.value}

    @mcp.tool(name="email_status")
    def email_status(request_id: str) -> dict:
        request = service.status(request_id)
        return {
            "id": str(request.id),
            "status": request.status.value,
            "to": request.to,
            "subject": request.subject,
        }

    @mcp.tool(name="email_list_recent")
    def email_list_recent(limit: int = 20) -> list[dict]:
        return [
            {
                "id": str(item.id),
                "status": item.status.value,
                "to": item.to,
                "subject": item.subject,
            }
            for item in service.list_recent(limit=limit)
        ]
