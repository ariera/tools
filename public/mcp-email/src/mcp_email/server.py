from mcp.server.fastmcp import FastMCP

from mcp_email.config import Settings
from mcp_email.store import SQLiteEmailStore
from mcp_email.tools.email import register_email_tools


def create_mcp(*, settings: Settings | None = None) -> FastMCP:
    settings = settings or Settings()
    store = SQLiteEmailStore(settings.store_path)
    store.initialize()
    mcp = FastMCP("EMBO Email", instructions="Restricted plain-text email tools for allowlisted recipients only.")
    register_email_tools(mcp, settings=settings, store=store)
    return mcp


def main() -> None:
    create_mcp().run()
