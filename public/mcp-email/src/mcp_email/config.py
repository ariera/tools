import os
from pathlib import Path
from typing import Annotated, Optional
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from pydantic import EmailStr, Field, field_validator, model_validator
from pydantic_settings import BaseSettings, NoDecode, SettingsConfigDict
from pydantic_settings.sources import ENV_FILE_SENTINEL, DotenvType


def _discover_project_root() -> Path:
    module_path = Path(__file__).resolve()
    for parent in module_path.parents:
        if (parent / "pyproject.toml").exists():
            return parent
    return module_path.parents[2]


PROJECT_ROOT = _discover_project_root()


class Settings(BaseSettings):
    model_config = SettingsConfigDict(env_file=None, extra="ignore")

    smtp_host: str
    smtp_port: int = Field(ge=1, le=65535)
    smtp_username: str
    smtp_password: str
    smtp_use_starttls: bool = True
    smtp_use_ssl: bool = False
    sender_email: EmailStr
    sender_name: str = "EMBO Email"
    allowed_recipients: Annotated[list[EmailStr], NoDecode]
    approval_required: bool = True
    admin_email: Optional[EmailStr] = None
    approval_token_ttl_hours: int = Field(default=24, ge=1, le=168)
    daily_limit: int = Field(default=10, ge=1, le=1000)
    throttle_seconds: int = Field(default=60, ge=1, le=86400)
    smtp_timeout_seconds: int = Field(default=30, ge=1, le=300)
    quota_timezone: str = "UTC"
    subject_max_length: int = Field(default=160, ge=1, le=998)
    body_max_length: int = Field(default=5000, ge=1, le=50000)
    store_path: str = str(PROJECT_ROOT / "data" / "email.sqlite3")

    def __init__(self, _project_root: str | Path | None = None, **values: object) -> None:
        env_file = values.pop("_env_file", ENV_FILE_SENTINEL)
        project_root = self._resolve_project_root(_project_root, env_file)
        resolved_env_file = self._resolve_env_file(project_root, env_file)
        super().__init__(_env_file=resolved_env_file, **values)
        self.store_path = self._resolve_store_path(self.store_path, project_root)

    @classmethod
    def _resolve_project_root(
        cls,
        override: str | Path | None,
        env_file: DotenvType | None,
    ) -> Path:
        if override is not None:
            return Path(override).expanduser().resolve()

        env_root_override = os.getenv("MCP_EMAIL_PROJECT_ROOT")
        if env_root_override:
            return Path(env_root_override).expanduser().resolve()

        if env_file not in (None, ENV_FILE_SENTINEL):
            env_path = cls._coerce_env_file_path(env_file)
            if env_path is not None:
                return env_path.parent.resolve()

        return PROJECT_ROOT

    @classmethod
    def _resolve_env_file(cls, project_root: Path, env_file: DotenvType | None) -> DotenvType | None:
        if env_file == ENV_FILE_SENTINEL:
            return project_root / ".env"
        return env_file

    @staticmethod
    def _coerce_env_file_path(env_file: DotenvType | None) -> Path | None:
        if env_file is None:
            return None
        if isinstance(env_file, (str, Path)):
            return Path(env_file).expanduser().resolve()
        if isinstance(env_file, (list, tuple)) and env_file:
            first_path = env_file[0]
            return Path(first_path).expanduser().resolve()
        return None

    @field_validator("smtp_host", "smtp_username", "smtp_password")
    @classmethod
    def require_non_blank_string(cls, value: str) -> str:
        stripped = value.strip()
        if not stripped:
            raise ValueError("value must not be blank")
        return stripped

    @field_validator("allowed_recipients", mode="before")
    @classmethod
    def normalize_allowlist(cls, value: object) -> list[str]:
        if isinstance(value, str):
            value = value.split(",")
        if not isinstance(value, list):
            raise TypeError("allowed_recipients must be a list or comma-separated string")
        return [item.strip().lower() for item in value if item.strip()]

    @field_validator("quota_timezone")
    @classmethod
    def validate_quota_timezone(cls, value: str) -> str:
        try:
            ZoneInfo(value)
        except ZoneInfoNotFoundError as exc:
            raise ValueError("quota_timezone must be a valid IANA timezone") from exc
        return value

    @classmethod
    def _resolve_store_path(cls, value: str, project_root: Path) -> str:
        path = Path(value).expanduser()
        if path.is_absolute():
            return str(path)
        return str((project_root / path).resolve())

    @model_validator(mode="after")
    def validate_transport_security(self) -> "Settings":
        if self.smtp_use_ssl and self.smtp_use_starttls:
            raise ValueError("smtp_use_ssl and smtp_use_starttls cannot both be enabled")
        if not self.smtp_use_ssl and not self.smtp_use_starttls:
            raise ValueError("smtp transport must use SSL or STARTTLS")
        return self
