from mcp_email.models import EmailStatus


class EmailDispatcher:
    def __init__(self, *, store, transport):
        self.store = store
        self.transport = transport

    def dispatch_once(self) -> None:
        request = self.store.claim_next_ready_request()
        if request is None:
            return
        try:
            message_id = self.transport.send_plain_text(
                to=request.to,
                subject=request.subject,
                body_text=request.body_text,
            )
        except Exception as exc:
            self.store.update_status(str(request.id), EmailStatus.FAILED, error_message=str(exc))
            raise
        self.store.update_status(str(request.id), EmailStatus.SENT, transport_message_id=message_id)
