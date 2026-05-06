import sqlite3
from datetime import UTC, datetime

from mcp_email.approval import (
    build_approval_email_body,
    compute_token_expiry,
    generate_approval_token,
    normalize_token,
)
from mcp_email.config import Settings
from mcp_email.models import EmailStatus, EmailSubmitRequest, StoredEmailRequest
from mcp_email.policy import build_dedupe_fingerprint, validate_submit_request
from mcp_email.rate_limits import RateLimiter
from mcp_email.store import SQLiteEmailStore


class EmailToolService:
    MAX_RECENT_REQUESTS = 100

    def __init__(self, *, settings: Settings, store: SQLiteEmailStore, transport=None, now_fn=None):
        self.settings = settings
        self.store = store
        self.transport = transport
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

        approval_token = None
        approval_token_expires_at = None
        if self.settings.approval_required:
            approval_token = generate_approval_token()
            approval_token_expires_at = compute_token_expiry(
                now=now, ttl_hours=self.settings.approval_token_ttl_hours
            )

        stored = StoredEmailRequest(
            to=request.to,
            subject=request.subject,
            body_text=request.body_text,
            reason=request.reason,
            idempotency_key=request.request_idempotency_key,
            dedupe_fingerprint=fingerprint,
            approval_token=approval_token,
            approval_token_expires_at=approval_token_expires_at,
            status=(
                EmailStatus.PENDING_APPROVAL
                if self.settings.approval_required
                else EmailStatus.READY_TO_SEND
            ),
            created_at=now,
            updated_at=now,
        )
        try:
            result = self.store.create_or_get_request(stored)
        except sqlite3.IntegrityError:
            if request.request_idempotency_key is not None:
                existing = self.store.find_by_idempotency_key(request.request_idempotency_key)
                if existing is not None:
                    return existing
            existing = self.store.find_by_fingerprint(fingerprint)
            if existing is not None:
                return existing
            raise

        is_new = result.id == stored.id
        if (
            is_new
            and self.settings.approval_required
            and approval_token is not None
            and approval_token_expires_at is not None
            and self.settings.admin_email is not None
            and self.transport is not None
        ):
            body = build_approval_email_body(
                result,
                token=approval_token,
                expires_at=approval_token_expires_at,
            )
            self.transport.send_plain_text(
                to=self.settings.admin_email,
                subject=f"[APPROVAL REQUIRED] Draft email: {result.subject}",
                body_text=body,
            )

        return result

    def approve_by_token(self, token: str) -> StoredEmailRequest:
        normalized = normalize_token(token)
        request = self.store.find_by_approval_token(normalized)
        if request is None:
            raise ValueError("invalid or expired approval token")
        now = self.now_fn()
        if request.approval_token_expires_at is not None and request.approval_token_expires_at < now:
            raise ValueError("approval token has expired")
        if request.status is not EmailStatus.PENDING_APPROVAL:
            raise ValueError(
                f"email is not pending approval (current status: {request.status.value})"
            )
        return self.store.approve_request(str(request.id), approval_actor="token")

    def reject_by_id(self, request_id: str, reason: str) -> StoredEmailRequest:
        request = self.store.get_request(request_id)
        if request is None:
            raise KeyError(request_id)
        if request.status is not EmailStatus.PENDING_APPROVAL:
            raise ValueError(
                f"email is not pending approval (current status: {request.status.value})"
            )
        return self.store.reject_request(
            str(request.id),
            approval_actor="agent",
            approval_reason=reason,
        )

    def status(self, request_id: str) -> StoredEmailRequest:
        request = self.store.get_request(request_id)
        if request is None:
            raise KeyError(request_id)
        return request

    def list_recent(self, limit: int = 20) -> list[StoredEmailRequest]:
        if limit < 1 or limit > self.MAX_RECENT_REQUESTS:
            raise ValueError(f"limit must be between 1 and {self.MAX_RECENT_REQUESTS}")
        return self.store.list_recent(limit=limit)


def register_email_tools(
    mcp, *, settings: Settings, store: SQLiteEmailStore, transport=None
) -> None:
    service = EmailToolService(settings=settings, store=store, transport=transport)

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

    @mcp.tool(name="email_approve")
    def email_approve(token: str) -> dict:
        """Approve a pending email using the approval token sent to the admin.

        The approval token is a short code (e.g. K7MR-T2NX) delivered to the
        admin via the approval notification email. It is used exclusively to
        approve the email — it cannot be used to reject it.

        To reject an email, use email_reject with the email ID instead.
        """
        updated = service.approve_by_token(token)
        return {"id": str(updated.id), "status": updated.status.value}

    @mcp.tool(name="email_reject")
    def email_reject(request_id: str, reason: str) -> dict:
        """Reject a pending email using its email ID.

        The email ID is a UUID included in the approval notification email sent
        to the admin. Rejection requires the email ID, not the approval token.
        The approval token is exclusively for approving emails.
        """
        updated = service.reject_by_id(request_id, reason)
        return {"id": str(updated.id), "status": updated.status.value}

    @mcp.tool(name="email_status")
    def email_status(request_id: str) -> dict:
        request = service.status(request_id)
        result: dict = {
            "id": str(request.id),
            "status": request.status.value,
            "to": request.to,
            "subject": request.subject,
        }
        if request.approval_token_expires_at is not None:
            result["approval_token_expires_at"] = request.approval_token_expires_at.isoformat()
        return result

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
