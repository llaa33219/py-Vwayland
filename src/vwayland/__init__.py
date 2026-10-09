"""py-Vwayland: virtual Wayland compositors spawned from Python.

```python
import vwayland

with vwayland.spawn(width=1280, height=720, headless=True) as comp:
    comp.launch("my-gui-app")
    comp.screenshot().save("screen.png")
    comp.click(100, 200)
    comp.type_text("hello")
```

See the docs/ directory in the package repository for full documentation.
"""

from .core import (
    Compositor,
    CompositorInfo,
    Image,
    connect,
    list,
    runtime_root,
    spawn,
)
from .errors import (
    AppError,
    CompositorNotFoundError,
    CompositorStartError,
    CompositorTimeoutError,
    ProtocolError,
    VwaylandError,
)

__version__ = "0.2.0"

__all__ = [
    "spawn",
    "connect",
    "list",
    "runtime_root",
    "Compositor",
    "CompositorInfo",
    "Image",
    "VwaylandError",
    "CompositorStartError",
    "CompositorNotFoundError",
    "CompositorTimeoutError",
    "AppError",
    "ProtocolError",
    "__version__",
]
