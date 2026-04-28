# mcp-email

A restricted, auditible email MCP server for controlled communication. Designed for organizational use where email sending needs governance, approval workflows, and strict rate limiting.

## Features

- **Allowlisted Recipients**: Only configured recipients can receive emails
- **Optional Approval Workflow**: Emails can require manual approval before sending
- **Rate Limiting**: Configure daily limits and throttling between sends
- **Audit Trail**: SQLite database tracks all email requests and their status
- **Plain Text Only**: Enforces plain-text emails for security and simplicity
- **SMTP Configuration**: Flexible SMTP backend support
- **Admin CLI**: Command-line tool for managing emails and policies

## Installation

```bash
pip install mcp-email
```

Or with development dependencies:

```bash
pip install "mcp-email[dev]"
```

## Configuration

Create a `.env` file based on `.env.example`:

```bash
cp .env.example .env
```

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `SMTP_HOST` | Yes | SMTP server hostname |
| `SMTP_PORT` | Yes | SMTP server port (typically 587 or 465) |
| `SMTP_USERNAME` | Yes | SMTP authentication username |
| `SMTP_PASSWORD` | Yes | SMTP authentication password |
| `SMTP_USE_STARTTLS` | No | Enable STARTTLS (default: true) |
| `SENDER_EMAIL` | Yes | Email address to send from |
| `ALLOWED_RECIPIENTS` | Yes | Comma-separated list of allowed recipient emails |
| `APPROVAL_REQUIRED` | No | Require manual approval for all emails (default: false) |
| `DAILY_LIMIT` | No | Maximum emails per day per user (default: unlimited) |
| `THROTTLE_SECONDS` | No | Minimum seconds between sends (default: 0) |
| `QUOTA_TIMEZONE` | No | Timezone for daily quota reset (default: UTC) |
| `STORE_PATH` | No | Path to SQLite database file (default: ./data/email.sqlite3) |

## Usage

### MCP Server

Run the MCP server for use with Claude and other MCP clients:

```bash
mcp-email
```

The server exposes tools for:
- **submit_email**: Submit an email for sending (with optional approval workflow)
- **list_emails**: View all submitted emails and their status
- **get_email**: Get details of a specific email

### Admin CLI

Manage emails and configuration:

```bash
mcp-email-admin --help
```

Commands:
- List emails with filtering and status
- Approve/reject pending emails
- Force send or cancel emails
- View system configuration

## Email Workflow

1. **Submit**: User submits email via `submit_email` tool
2. **Validation**: Recipients checked against allowlist, subject/body validated
3. **Approval** (if enabled): Email stored as `PENDING_APPROVAL` awaiting admin review
4. **Queue**: Email moved to `READY_TO_SEND` status
5. **Send**: Email sent via SMTP, marked as `SENT` on success or `FAILED` on error
6. **Audit**: All state changes recorded in SQLite database

## Status Values

- `PENDING_APPROVAL`: Awaiting manual approval (if approval required)
- `APPROVED`: Approved and queued for sending
- `REJECTED`: Rejected during approval
- `READY_TO_SEND`: Validated and ready for delivery
- `SENDING`: Currently being sent
- `SENT`: Successfully delivered
- `FAILED`: Failed to send

## Rate Limiting

Two rate limits work together:

- **Daily Limit**: Maximum emails per day per user (resets daily in configured timezone)
- **Throttle Seconds**: Minimum seconds between any two email sends

Both are enforced per SMTP sender account.

## Development

Install development dependencies:

```bash
pip install -e ".[dev]"
```

Run tests:

```bash
pytest
```

Run tests with coverage:

```bash
pytest --cov=src/mcp_email
```

## Project Structure

```
src/mcp_email/
├── server.py           # MCP FastMCP server setup
├── models.py           # Pydantic models (EmailSubmitRequest, EmailStatus, etc)
├── config.py           # Settings and configuration management
├── store.py            # SQLite email store
├── dispatcher.py       # Email sending dispatcher
├── smtp_client.py      # SMTP client wrapper
├── policy.py           # Policy enforcement (allowlist, rate limits)
├── rate_limits.py      # Rate limiting logic
├── cli.py              # Admin CLI tool
└── tools/
    └── email.py        # MCP tool definitions
```

## Security Considerations

- **Allowlist Enforcement**: Only configured recipients can receive emails
- **Plain Text Only**: Prevents HTML injection attacks
- **Approval Workflow**: Optional human review before sending
- **Rate Limiting**: Prevents abuse and email flooding
- **Audit Trail**: Full record of all email activity
- **Idempotency Keys**: Prevents duplicate sends from retries

## License

See LICENSE file
