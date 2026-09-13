"""py-Vwayland exception hierarchy."""

from __future__ import annotations


class VwaylandError(Exception):
    """Base class for all py-Vwayland errors."""


class CompositorStartError(VwaylandError):
    """Failed to start the compositor process or wait for it to become ready."""


class CompositorNotFoundError(VwaylandError):
    """No compositor instance with the requested id."""


class CompositorTimeoutError(VwaylandError):
    """Timed out waiting for a compositor response/state."""


class AppError(VwaylandError):
    """Errors related to launching/closing the app inside a compositor."""


class ProtocolError(VwaylandError):
    """The compositor returned an error response or the protocol broke."""
