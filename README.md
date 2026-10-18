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

A change here is released as a new tag, and each installer moves to that tag.

## License

GPL-3.0-or-later.
