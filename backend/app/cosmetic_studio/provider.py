"""Provider boundary: transport submission is never a visual success ACK."""
from dataclasses import dataclass
from typing import Protocol


class CosmeticError(RuntimeError):
    pass


@dataclass(frozen=True)
class ProviderStatus:
    state: str
    visual_verified: bool | None
    verification_source: str | None


class SessionTransport(Protocol):
    """Implemented only by our exclusively owned offline session controller."""
    def validate_context(self, *, paused: bool) -> None: ...
    def load_extension(self) -> None: ...
    def request_refresh(self, profile_digest: str) -> None: ...
    def stop(self) -> None: ...


class SkinProvider(Protocol):
    def load(self) -> None: ...
    def apply_profile(self, profile_digest: str) -> None: ...
    def check_ready(self) -> bool: ...
    def get_status(self) -> ProviderStatus: ...
    def unload(self) -> None: ...


class NskinzProvider:
    """Fixed-version local adapter, with mandatory host-owned preview review.

    Neither a successful Windows event nor nSkinz field-readback text is a
    render acknowledgment. The service must bind human approval to its own
    fresh preview/session before calling confirm_preview. No hot-unload claim.
    """
    def __init__(self, transport: SessionTransport):
        self._transport = transport
        self._state = "new"
        self._confirmed = False

    def load(self) -> None:
        if self._state != "new":
            raise CosmeticError("PROVIDER_ALREADY_USED")
        self._transport.validate_context(paused=True)
        self._transport.load_extension()
        self._state = "loaded"

    def apply_profile(self, profile_digest: str) -> None:
        if self._state != "loaded":
            raise CosmeticError("PROVIDER_NOT_LOADED")
        self._transport.validate_context(paused=True)
        self._transport.request_refresh(profile_digest)
        self._state = "pending_visual_review"

    def confirm_preview(self) -> None:
        if self._state != "pending_visual_review":
            raise CosmeticError("VISUAL_REVIEW_NOT_PENDING")
        self._transport.validate_context(paused=True)
        self._confirmed = True
        self._state = "human_confirmed"

    def check_ready(self) -> bool:
        if self._state != "human_confirmed" or not self._confirmed:
            return False
        self._transport.validate_context(paused=True)
        return True

    def get_status(self) -> ProviderStatus:
        return ProviderStatus(self._state, True if self._confirmed else None,
                              "human_preview" if self._confirmed else None)

    def unload(self) -> None:
        self._confirmed = False
        self._state = "stopping"
        # The controller must verify owned process exit; closing nSkinz UI
        # or merely issuing quit does not satisfy this operation.
        self._transport.stop()
        self._state = "stopped"
