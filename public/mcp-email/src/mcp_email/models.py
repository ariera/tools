from enum import Enum
from typing import Optional
from uuid import UUID, uuid4

from pydantic import AwareDatetime, BaseModel, ConfigDict, EmailStr, Field, field_validator


class EmailStatus(str, Enum):
    PENDING_APPROVAL = "pending_approval"
    APPROVED = "approved"
    REJECTED = "rejected"
    READY_TO_SEND = "ready_to_send"
    SENDING = "sending"
    SENT = "sent"
    FAILED = "failed"


class EmailSubmitRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    to: EmailStr
    subject: str
    body_text: str
    reason: Optional[str] = None
    request_idempotency_key: Optional[str] = None

    @field_validator("subject")
    @classmethod
    def normalize_subject(cls, value: str) -> str:
        stripped = value.strip()
        if not stripped:
            raise ValueError("subject must not be blank")
        if "\r" in stripped or "\n" in stripped:
            raise ValueError("subject must not contain CR or LF characters")
        return stripped

    @field_validator("body_text")
    @classmethod
    def require_non_blank_body(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("body_text must not be blank")
        return value

    @field_validator("reason", "request_idempotency_key")
    @classmethod
    def normalize_optional_text(cls, value: Optional[str]) -> Optional[str]:
        if value is None:
            return None
        stripped = value.strip()
        return stripped or None


class StoredEmailRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    id: UUID = Field(default_factory=uuid4)
    to: EmailStr
    subject: str
    body_text: str
    reason: Optional[str] = None
    idempotency_key: Optional[str] = None
    dedupe_fingerprint: str
    status: EmailStatus
    created_at: AwareDatetime
    updated_at: AwareDatetime
    approval_actor: Optional[str] = None
    approval_reason: Optional[str] = None
    transport_message_id: Optional[str] = None
    error_message: Optional[str] = None

    @field_validator("subject")
    @classmethod
    def normalize_subject(cls, value: str) -> str:
        stripped = value.strip()
        if not stripped:
            raise ValueError("subject must not be blank")
        if "\r" in stripped or "\n" in stripped:
            raise ValueError("subject must not contain CR or LF characters")
        return stripped

    @field_validator("body_text")
    @classmethod
    def require_non_blank_body(cls, value: str) -> str:
        if not value.strip():
            raise ValueError("body_text must not be blank")
        return value

    @field_validator("reason", "idempotency_key")
    @classmethod
    def normalize_optional_text(cls, value: Optional[str]) -> Optional[str]:
        if value is None:
            return None
        stripped = value.strip()
        return stripped or None

    @field_validator("dedupe_fingerprint")
    @classmethod
    def normalize_dedupe_fingerprint(cls, value: str) -> str:
        stripped = value.strip()
        if not stripped:
            raise ValueError("dedupe_fingerprint must not be blank")
        return stripped
