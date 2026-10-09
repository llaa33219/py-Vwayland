# CLI Reference

The `vwayland` command exposes the same features as the Python API in a shell.
Installing the package registers the `vwayland` entry point (location depends on
your pip install path, e.g. `~/.local/bin`).

```console
$ vwayland --help
$ vwayland --version
```

Common rules:

- Commands that report state print JSON to stdout.
- Errors are printed to stderr as `vwayland: error: ...` with exit code 1.
- Compositors spawned via `spawn` keep running after the CLI exits
  (created with `kill_on_exit=False`). Terminate them explicitly with `kill`.

## spawn — spawn a compositor

```console
$ vwayland spawn [--width W] [--height H] [--headless|--windowed] [--id ID]
```

Defaults: 1280x720, `--headless`. Prints the same JSON as `ping` once ready.

```console
$ vwayland spawn --width 1920 --height 1080 --id demo
```

## list — list compositors

```console
$ vwayland list
```

Prints every running instance as a JSON array
(`id`, `pid`, `runtime_dir`, `display`, `width`, `height`, `headless`, `app_pid`).

## kill — terminate compositors

```console
$ vwayland kill <ID|all>
```

`all` terminates every compositor shown by `list`.

## launch — open a program

```console
$ vwayland launch <ID> -- <command> [args...]
```

Runs the command after `--` inside the compositor. Prints the app pid as JSON.
Only one app per compositor.

```console
$ vwayland launch demo -- foot --title test
```

## close-app — close the program

```console
$ vwayland close-app <ID>
```

Sends SIGTERM and waits. Prints `{"closed": true|false}`.

## resize — change the screen size

```console
$ vwayland resize <ID> <WIDTH> <HEIGHT>
```

## screenshot — capture the screen

```console
$ vwayland screenshot <ID> [-o file.png]
```

Saves to `<ID>.png` when `-o` is omitted.

## click / move / drag / scroll — mouse

```console
$ vwayland click <ID> <X> <Y> [--button left]
$ vwayland move <ID> <X> <Y>
$ vwayland drag <ID> <X1> <Y1> <X2> <Y2> [--button left] [--steps 20]
$ vwayland scroll <ID> [--dx N] [--dy N]
```

Coordinates are logical pixels (floats allowed); the scroll unit is wheel
detents, and `dy > 0` scrolls up.

## key / combo / type — keyboard

```console
$ vwayland key <ID> <key-name>       # e.g.: enter, a, f5, tab
$ vwayland combo <ID> <keys...>      # e.g.: vwayland combo demo ctrl c
$ vwayland type <ID> <text>          # US-layout typing
```

See the [key name table](api-python.md#key-name-table) in the Python API docs.

## paste — paste text via the clipboard

```console
$ vwayland paste <ID> <text> [--no-restore] [--restore-delay 0.15]
```

Pastes `text` into the focused field: back up the clipboard, set it, press
Ctrl+V, then restore the backup. Works for **any language** (Korean, emoji, ...)
and is the way to go when the app blocks typing/pasting shortcuts — unlike
`type`, which always goes through the keyboard.

- `--no-restore`: leave `text` in the clipboard instead of restoring the backup.
- `--restore-delay`: seconds to wait between Ctrl+V and the restore
  (default `0.15`, gives the app time to request the paste data).

```console
$ vwayland paste demo "안녕하세요"
```

## clipboard-get / clipboard-set / clipboard-clear — clipboard

```console
$ vwayland clipboard-get <ID>            # print the clipboard text
$ vwayland clipboard-set <ID> <text>     # replace the clipboard content
$ vwayland clipboard-clear <ID>          # clear the clipboard
```

`clipboard-get` prints the raw text followed by a newline, and prints an empty
line (exit code 0) when the clipboard holds no text. `clipboard-set` accepts any
Unicode text.

## set-headless — toggle headless mode

```console
$ vwayland set-headless <ID> <on|off>
```

Restarts the compositor under the same id. Any running app is terminated.

## ping — query status

```console
$ vwayland ping <ID>
```

Prints `ok`, `version`, `id`, `display`, `width`, `height`, `headless`,
`app_pid` as JSON.

## Example session

```console
$ vwayland spawn --id demo --width 1280 --height 720
$ vwayland launch demo -- my-gui-app
$ vwayland click demo 300 200
$ vwayland type demo "hello"
$ vwayland paste demo "안녕하세요"
$ vwayland key demo enter
$ vwayland screenshot demo -o shot.png
$ vwayland kill demo
```
