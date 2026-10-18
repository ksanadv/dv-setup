# dv-setup

The setup window that the dv apps' installers share: a choose page with the app's options, a progress page and a done
page. It is drawn with GDI and follows the system's light or dark theme and accent colour. It installs per user, with
no administrator rights and no UI framework.

Used by [dvfiles](https://github.com/ksanadv/dv-folder), [dvnote](https://github.com/ksanadv/dvnote) and
[dvsnip](https://github.com/ksanadv/dvsnip).

## Use

```toml
[dependencies]
dv-setup = { git = "https://github.com/ksanadv/dv-setup", tag = "v0.1.0" }
```

An installer describes its app in a `dv_setup::Product` (name, version, install folder, the installed version, its
checkboxes) and does the install in a `dv_setup::Work`. The `install` method gets which boxes are ticked and a `step`
callback for the progress page. `dv_setup::run(product, work)` shows the window.

By convention every installer also takes these switches:
- `--quiet` installs without the window.
- `--preview` walks through the pages with a `Work` that only waits, so nothing is installed.

Set `<APP>_NOACTIVATE=1` when testing so the window doesn't take the keyboard.

A change here is released as a new tag, and each installer moves to that tag. The `net` feature adds the GitHub
release lookup and the checked download (WinHTTP, BCrypt).

## dv-setup.exe: all the apps at once

`suite/` builds `dv-setup.exe`, one setup for all the dv apps:
1. It looks up each app's latest release on GitHub and ticks the apps that are missing or older than their release.
2. For each ticked app, it downloads that app's own setup program and checks it against the published SHA-256.
3. It runs that setup with `--quiet`, so each app gets its usual options.

It registers nothing itself: every app has its own entry in Apps & features. An app with no release yet isn't
listed. `--quiet` installs without the window. `--preview` downloads and checks the setups but runs nothing.

```powershell
cd suite; cargo build --release    # dv-setup.exe
```

## License

GPL-3.0-or-later.
