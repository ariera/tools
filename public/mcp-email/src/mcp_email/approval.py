import secrets
from datetime import UTC, datetime, timedelta

from mcp_email.models import StoredEmailRequest

_ALPHABET = "ABCDEFGHJKMNPQRSTVWXYZ23456789"  # Crockford base-32: no O/0, I/1, L/U confusables


def generate_approval_token() -> str:
    part = lambda: "".join(secrets.choice(_ALPHABET) for _ in range(4))
    return f"{part()}-{part()}"


def normalize_token(token: str) -> str:
    return token.strip().upper()


def compute_token_expiry(*, now: datetime, ttl_hours: int) -> datetime:
    return now + timedelta(hours=ttl_hours)


def build_approval_email_body(
    request: StoredEmailRequest,
    *,
    token: str,
    expires_at: datetime,
) -> str:
    expires_str = expires_at.astimezone(UTC).strftime("%Y-%m-%d %H:%M UTC")
    email_id = str(request.id)
    bar = "━" * 50
    thin = "─" * 50

    lines = [
        "[DRAFT AWAITING APPROVAL]",
        "",
        "An AI agent has requested to send the following email.",
        "Please review and approve or reject.",
        "",
        bar,
        f"APPROVAL TOKEN  : {token}",
        f"Email ID        : {email_id}",
        f"Token expires   : {expires_str}",
        bar,
        "",
        "TO APPROVE — give the AI agent the approval token:",
        f'  "Approve {token}"',
        "",
        "TO REJECT — give the AI agent the email ID:",
        f'  "Reject {email_id}"',
        "",
        "IMPORTANT: The approval token is a one-time secret used",
        "exclusively to APPROVE this email. It cannot be used to",
        "reject it. Rejection always requires the email ID above.",
        "",
        thin,
        "DRAFT EMAIL DETAILS",
        thin,
        f"To      : {request.to}",
        f"Subject : {request.subject}",
    ]
    if request.reason:
        lines.append(f"Reason  : {request.reason}")
    lines += [
        thin,
        "",
        request.body_text,
        "",
        thin,
    ]
    return "\n".join(lines)
