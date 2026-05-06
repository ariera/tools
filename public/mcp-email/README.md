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
| `APPROVAL_REQUIRED` | No | Require manual approval for all emails (default: true) |
| `ADMIN_EMAIL` | No | Email address that receives approval request notifications (required when `APPROVAL_REQUIRED=true` to use the token workflow) |
| `APPROVAL_TOKEN_TTL_HOURS` | No | Hours before an approval token expires (default: 24, max: 168) |
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
- **email_submit**: Submit an email for sending (triggers approval workflow if enabled)
- **email_approve**: Approve a pending email using the approval token
- **email_reject**: Reject a pending email using the email ID
- **email_status**: Get the status and details of a specific email
- **email_list_recent**: List recent email requests

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

### Without approval (`APPROVAL_REQUIRED=false`)

1. **Submit**: Agent calls `email_submit`
2. **Validation**: Recipient checked against allowlist, subject/body validated
3. **Send**: Email sent via SMTP, marked `SENT` on success or `FAILED` on error

### With approval (`APPROVAL_REQUIRED=true`)

1. **Submit**: Agent calls `email_submit`
2. **Validation**: Recipient checked against allowlist, subject/body validated
3. **Notify**: A notification email is sent to `ADMIN_EMAIL` containing:
   - A short **approval token** (e.g. `K7MR-T2NX`) at the top
   - The email ID (UUID)
   - Full draft details: recipient, subject, body, reason
   - Token expiry time
4. **Admin review**: Admin reads the notification email and decides
5. **Approve**: Admin gives the approval token to the AI agent (e.g. `"Approve K7MR-T2NX"`). The agent calls `email_approve(token="K7MR-T2NX")`. The server validates the token and queues the email for sending.
6. **Reject**: Admin gives the email ID to the AI agent (e.g. `"Reject <id>"`). The agent calls `email_reject(request_id="<id>", reason="...")`. The email is permanently rejected.
7. **Send**: Approved email is delivered via SMTP.

### Token vs. Email ID

| Action | What to use |
|--------|-------------|
| **Approve** | Approval token (`XXXX-XXXX`) |
| **Reject** | Email ID (UUID) |

The approval token is a one-time secret used **exclusively to approve** an email. It cannot be used to reject. This separation ensures that sharing the token with the AI agent can only result in approval — the admin must make a separate, deliberate decision to reject using the ID.

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
├── approval.py         # Approval token generation and notification email builder
├── cli.py              # Admin CLI tool
└── tools/
    └── email.py        # MCP tool definitions
```

## Security Considerations

- **Allowlist Enforcement**: Only configured recipients can receive emails
- **Plain Text Only**: Prevents HTML injection attacks
- **Approval Workflow**: Optional human review before sending; token design ensures the AI agent cannot approve emails without a human explicitly handing it the token
- **Token/ID Separation**: The approval token approves; the email ID rejects. They are different values, so a token leak can only result in approval, not rejection
- **Short-lived Tokens**: Approval tokens expire after a configurable TTL (default 24 h), preventing stale approvals
- **Rate Limiting**: Prevents abuse and email flooding
- **Audit Trail**: Full record of all email activity
- **Idempotency Keys**: Prevents duplicate sends from retries

## License

See LICENSE file
