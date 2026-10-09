"""vwayland CLI: control compositors from the shell."""

from __future__ import annotations

import argparse
import json
import sys

from . import core
from .errors import VwaylandError
from . import __version__


def _print_json(obj) -> None:
    print(json.dumps(obj, ensure_ascii=False, indent=2, default=str))


def _build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="vwayland",
        description="Virtual Wayland compositor control CLI (py-Vwayland)",
    )
    p.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    sub = p.add_subparsers(dest="action", required=True)

    sp = sub.add_parser("spawn", help="spawn a compositor")
    sp.add_argument("--width", type=int, default=1280)
    sp.add_argument("--height", type=int, default=720)
    mode = sp.add_mutually_exclusive_group()
    mode.add_argument("--headless", dest="headless", action="store_true", default=True)
    mode.add_argument("--windowed", dest="headless", action="store_false")
    sp.add_argument("--id", default=None)

    sub.add_parser("list", help="list running compositors")

    kp = sub.add_parser("kill", help="terminate compositor(s)")
    kp.add_argument("id", help="instance id or 'all'")

    lp = sub.add_parser("launch", help="open a program inside a compositor")
    lp.add_argument("id")
    lp.add_argument("argv", nargs=argparse.REMAINDER, help="command to run (e.g.: vwayland launch ID -- foot)")

    cp = sub.add_parser("close-app", help="close the program inside a compositor")
    cp.add_argument("id")

    rp = sub.add_parser("resize", help="change the screen size")
    rp.add_argument("id")
    rp.add_argument("width", type=int)
    rp.add_argument("height", type=int)

    sp2 = sub.add_parser("screenshot", help="capture the screen (PNG)")
    sp2.add_argument("id")
    sp2.add_argument("-o", "--output", default=None, help="output file (default: <id>.png)")

    mp = sub.add_parser("click", help="click at coordinates")
    mp.add_argument("id")
    mp.add_argument("x", type=float)
    mp.add_argument("y", type=float)
    mp.add_argument("--button", default="left")

    mv = sub.add_parser("move", help="move the pointer")
    mv.add_argument("id")
    mv.add_argument("x", type=float)
    mv.add_argument("y", type=float)

    dp = sub.add_parser("drag", help="drag")
    dp.add_argument("id")
    dp.add_argument("x1", type=float)
    dp.add_argument("y1", type=float)
    dp.add_argument("x2", type=float)
    dp.add_argument("y2", type=float)
    dp.add_argument("--button", default="left")
    dp.add_argument("--steps", type=int, default=20)

    sc = sub.add_parser("scroll", help="scroll (wheel detents)")
    sc.add_argument("id")
    sc.add_argument("--dx", type=float, default=0.0)
    sc.add_argument("--dy", type=float, default=0.0)

    ky = sub.add_parser("key", help="key input (e.g.: enter, a, f5)")
    ky.add_argument("id")
    ky.add_argument("key")

    cb = sub.add_parser("combo", help="key combo (e.g.: ctrl c)")
    cb.add_argument("id")
    cb.add_argument("keys", nargs="+")

    tp = sub.add_parser("type", help="type a string (any language, e.g. Korean)")
    tp.add_argument("id")
    tp.add_argument("text")

    pas = sub.add_parser(
        "paste", help="paste text via clipboard (any language, e.g. Korean)"
    )
    pas.add_argument("id")
    pas.add_argument("text")
    pas.add_argument(
        "--no-restore",
        action="store_true",
        help="leave the pasted text in the clipboard instead of restoring the backup",
    )
    pas.add_argument(
        "--restore-delay",
        type=float,
        default=0.15,
        help="seconds between Ctrl+V and the clipboard restore (default: 0.15)",
    )

    cgp = sub.add_parser("clipboard-get", help="print the clipboard text")
    cgp.add_argument("id")

    csp = sub.add_parser("clipboard-set", help="set the clipboard text")
    csp.add_argument("id")
    csp.add_argument("text")

    ccp = sub.add_parser("clipboard-clear", help="clear the clipboard")
    ccp.add_argument("id")

    hp = sub.add_parser("set-headless", help="toggle headless mode (app is terminated)")
    hp.add_argument("id")
    hp.add_argument("mode", choices=["on", "off"])

    pp = sub.add_parser("ping", help="query status")
    pp.add_argument("id")
    return p


def main(argv: "list[str] | None" = None) -> int:
    args = _build_parser().parse_args(argv)
    try:
        if args.action == "spawn":
            comp = core.spawn(
                width=args.width,
                height=args.height,
                headless=args.headless,
                id=args.id,
                kill_on_exit=False,  # CLI-spawned compositors outlive the CLI process
            )
            _print_json(comp.info())
        elif args.action == "list":
            _print_json([vars(i) for i in core.list()])
        elif args.action == "kill":
            if args.id == "all":
                for info in core.list():
                    core.connect(info.id).kill()
                    print(f"killed {info.id}")
            else:
                core.connect(args.id).kill()
                print(f"killed {args.id}")
        elif args.action == "launch":
            argv_cmd = args.argv
            if argv_cmd and argv_cmd[0] == "--":
                argv_cmd = argv_cmd[1:]
            if not argv_cmd:
                raise VwaylandError("launch requires a command: vwayland launch ID -- cmd args...")
            pid = core.connect(args.id).launch(argv_cmd)
            _print_json({"pid": pid})
        elif args.action == "close-app":
            closed = core.connect(args.id).close_app()
            _print_json({"closed": closed})
        elif args.action == "resize":
            w, h = core.connect(args.id).resize(args.width, args.height)
            _print_json({"width": w, "height": h})
        elif args.action == "screenshot":
            img = core.connect(args.id).screenshot()
            out = args.output or f"{args.id}.png"
            img.save(out)
            _print_json({"path": out, "width": img.width, "height": img.height})
        elif args.action == "click":
            core.connect(args.id).click(args.x, args.y, button=args.button)
        elif args.action == "move":
            core.connect(args.id).move_to(args.x, args.y)
        elif args.action == "drag":
            core.connect(args.id).drag(
                args.x1, args.y1, args.x2, args.y2,
                button=args.button, steps=args.steps,
            )
        elif args.action == "scroll":
            core.connect(args.id).scroll(dx=args.dx, dy=args.dy)
        elif args.action == "key":
            core.connect(args.id).key(args.key)
        elif args.action == "combo":
            core.connect(args.id).combo(*args.keys)
        elif args.action == "type":
            core.connect(args.id).type_text(args.text)
        elif args.action == "paste":
            core.connect(args.id).paste_text(
                args.text,
                restore=not args.no_restore,
                restore_delay=args.restore_delay,
            )
        elif args.action == "clipboard-get":
            text = core.connect(args.id).clipboard_get()
            print("" if text is None else text)
        elif args.action == "clipboard-set":
            core.connect(args.id).clipboard_set(args.text)
        elif args.action == "clipboard-clear":
            core.connect(args.id).clipboard_clear()
        elif args.action == "set-headless":
            core.connect(args.id).set_headless(args.mode == "on")
        elif args.action == "ping":
            _print_json(core.connect(args.id).info())
    except VwaylandError as e:
        print(f"vwayland: error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
