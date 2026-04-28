import argparse
import getpass
import os
import sqlite3
import sys
from pathlib import Path

from mcp_email.config import PROJECT_ROOT
from mcp_email.models import EmailStatus
from mcp_email.store import SQLiteEmailStore


DEFAULT_STORE_PATH = PROJECT_ROOT / "data" / "email.sqlite3"


class CommandError(RuntimeError):
    pass


def _default_actor() -> str:
    actor = getpass.getuser() or os.getenv("USER") or os.getenv("USERNAME")
    return actor or "unknown"


def _resolve_store_path(value: str | None) -> Path:
    if value:
        return Path(value).expanduser().resolve()
    env_value = os.getenv("MCP_EMAIL_STORE_PATH") or os.getenv("STORE_PATH")
    if env_value:
        return Path(env_value).expanduser().resolve()
    return DEFAULT_STORE_PATH


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="mcp-email-admin")
    parser.add_argument("--store-path", help="Path to the SQLite email request store.")
    subparsers = parser.add_subparsers(dest="command", required=True)

    approve = subparsers.add_parser("approve", help="Approve a pending request.")
    approve.add_argument("request_id")
    approve.add_argument("--actor", default=_default_actor())
    approve.add_argument("--reason")

    reject = subparsers.add_parser("reject", help="Reject a pending request.")
    reject.add_argument("request_id")
    reject.add_argument("--actor", default=_default_actor())
    reject.add_argument("--reason", required=True)

    subparsers.add_parser("list-pending", help="List requests waiting for approval.")
    return parser


def _load_store(store_path: str | None) -> SQLiteEmailStore:
    store = SQLiteEmailStore(_resolve_store_path(store_path))
    store.initialize()
    return store


def _require_request(store: SQLiteEmailStore, request_id: str):
    request = store.get_request(request_id)
    if request is None:
        raise CommandError(f"request not found: {request_id}")
    return request


def _require_pending_request(store: SQLiteEmailStore, request_id: str):
    request = _require_request(store, request_id)
    if request.status is not EmailStatus.PENDING_APPROVAL:
        raise CommandError(
            f"request {request_id} is not pending approval (current status: {request.status.value})"
        )
    return request


def _approve_request(store: SQLiteEmailStore, *, request_id: str, actor: str, reason: str | None) -> None:
    request = _require_pending_request(store, request_id)
    try:
        updated = store.approve_request(
            str(request.id),
            approval_actor=actor,
            approval_reason=reason,
        )
    except ValueError as exc:
        raise CommandError(str(exc)) from exc
    print(f"approved {updated.id} -> {updated.status.value}")


def _reject_request(store: SQLiteEmailStore, *, request_id: str, actor: str, reason: str) -> None:
    request = _require_pending_request(store, request_id)
    try:
        updated = store.reject_request(
            str(request.id),
            approval_actor=actor,
            approval_reason=reason,
        )
    except ValueError as exc:
        raise CommandError(str(exc)) from exc
    print(f"rejected {updated.id} -> {updated.status.value}")


def _list_pending(store: SQLiteEmailStore) -> None:
    for request in store.list_by_status(EmailStatus.PENDING_APPROVAL):
        print(
            f"{request.id}\t{request.created_at.isoformat()}\t{request.to}\t{request.subject}"
        )


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)

    try:
        store = _load_store(args.store_path)
    except (OSError, sqlite3.Error) as exc:
        print(f"failed to initialize store: {exc}", file=sys.stderr)
        return 1

    try:
        if args.command == "approve":
            _approve_request(
                store,
                request_id=args.request_id,
                actor=args.actor,
                reason=args.reason,
            )
        elif args.command == "reject":
            _reject_request(
                store,
                request_id=args.request_id,
                actor=args.actor,
                reason=args.reason,
            )
        elif args.command == "list-pending":
            _list_pending(store)
        else:
            raise CommandError(f"unknown command: {args.command}")
    except sqlite3.Error as exc:
        print(f"command failed: {exc}", file=sys.stderr)
        return 1
    except CommandError as exc:
        print(str(exc), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
