import hashlib
from datetime import UTC, datetime

from mcp_email.config import Settings
from mcp_email.models import EmailSubmitRequest


class PolicyViolation(ValueError):
    pass


def _contains_header_break(value: str) -> bool:
    return "\n" in value or "\r" in value


def validate_submit_request(
    request: EmailSubmitRequest, settings: Settings
) -> EmailSubmitRequest:
    if _contains_header_break(request.to):
        raise PolicyViolation("Recipient contains header injection characters")
    normalized_to = request.to.strip().lower()
    if normalized_to not in settings.allowed_recipients:
        raise PolicyViolation(f"Recipient {normalized_to} is not allowlisted")
    if _contains_header_break(request.subject):
        raise PolicyViolation("Subject contains header injection characters")
    if len(request.subject) > settings.subject_max_length:
        raise PolicyViolation("Subject exceeds configured limit")
    if len(request.body_text) > settings.body_max_length:
        raise PolicyViolation("Body exceeds configured limit")
    return request.model_copy(update={"to": normalized_to})


def build_dedupe_fingerprint(
    request: EmailSubmitRequest,
    *,
    now: datetime | None = None,
    window_seconds: int | None = None,
) -> str:
    payload = "\n".join(
        [
            request.to.strip().lower(),
            request.subject.strip(),
            request.body_text.strip(),
        ]
    )
    digest = hashlib.sha256(payload.encode("utf-8")).hexdigest()
    if now is None:
        return digest
    if window_seconds is None:
        raise ValueError("window_seconds is required when now is provided")
    bucket = int(now.astimezone(UTC).timestamp()) // window_seconds
    return f"{bucket}:{digest}"
