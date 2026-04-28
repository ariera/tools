from dataclasses import dataclass
from datetime import datetime
from zoneinfo import ZoneInfo

from mcp_email.store import SQLiteEmailStore


class QuotaExceeded(ValueError):
    pass


@dataclass(slots=True)
class Reservation:
    recipient: str
    dedupe_fingerprint: str
    reserved_at: datetime


class RateLimiter:
    def __init__(
        self,
        *,
        store: SQLiteEmailStore,
        daily_limit: int,
        throttle_seconds: int,
        timezone_name: str,
    ):
        self.store = store
        self.daily_limit = daily_limit
        self.throttle_seconds = throttle_seconds
        self.timezone = ZoneInfo(timezone_name)

    def reserve_slot(
        self, recipient: str, dedupe_fingerprint: str, *, now: datetime
    ) -> Reservation:
        quota_day = now.astimezone(self.timezone).date().isoformat()
        error = self.store.reserve_quota_slot(
            quota_day=quota_day,
            reserved_at=now,
            dedupe_fingerprint=dedupe_fingerprint,
            daily_limit=self.daily_limit,
            throttle_seconds=self.throttle_seconds,
        )
        if error is not None:
            raise QuotaExceeded(error)
        return Reservation(
            recipient=recipient,
            dedupe_fingerprint=dedupe_fingerprint,
            reserved_at=now,
        )
