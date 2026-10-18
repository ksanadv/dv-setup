// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 dv-subsystem-set contributors
//! One setup for all the dv apps: it looks up each app's latest release on GitHub, ticks the ones that are missing or
//! out of date, and installs each by downloading its own setup program, checking it against its published SHA-256 and
//! running it with `--quiet`. It registers nothing itself: each app has its own entry in Apps & features.
//!
//! `--quiet` installs the ticked apps with no window. `--preview` downloads and checks them but runs nothing.
#![windows_subsystem = "windows"]

use dv_setup::{net, Choice, Product, Scope, Work};
use std::path::{Path, PathBuf};

const AGENT: &str = concat!("dv-setup/", env!("CARGO_PKG_VERSION"));

struct App {
    name: &'static str,
    repo: &'static str,
    /// the Windows program it stands in for
    instead_of: &'static str,
}

const APPS: [App; 3] = [
    App { name: "dvfiles", repo: "ksanadv/dv-folder", instead_of: "File Explorer" },
    App { name: "dvnote", repo: "ksanadv/dvnote", instead_of: "Notepad" },
    App { name: "dvsnip", repo: "ksanadv/dvsnip", instead_of: "the Snipping Tool" },
];

/// an app with a release: what is installed and what the latest release is
struct Offer {
    app: &'static App,
    installed: Option<String>,
    latest: String,
}

impl Offer {
    /// missing, or older than the release
    fn wanted(&self) -> bool {
        self.installed.as_deref().is_none_or(|i| dv_setup::is_newer(&self.latest, i))
    }

    /// "dvnote 0.3.2, for Notepad (0.3.0 is installed)"
    fn label(&self) -> String {
        let App { name, instead_of, .. } = self.app;
        let state = match self.installed.as_deref() {
            None => String::new(),
            Some(i) if i == self.latest => " (installed, up to date)".into(),
            Some(i) => format!(" ({i} is installed)"),
        };
        format!("{name} {}, for {instead_of}{state}", self.latest)
    }
}

/// the version Apps & features lists for `name`, which each app's setup writes
fn installed(name: &str) -> Option<String> {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{name}"))
        .ok()?
        .get_value("DisplayVersion")
        .ok()
}

/// every app's latest release, looked up side by side; the apps with one, and a line about the others
fn offers() -> (Vec<Offer>, Option<String>) {
    let found: Vec<_> = std::thread::scope(|s| {
        let jobs: Vec<_> = APPS.iter().map(|a| s.spawn(move || (a, net::latest(a.repo, AGENT)))).collect();
        jobs.into_iter().map(|j| j.join().expect("a release lookup does not panic")).collect()
    });
    let (mut offers, mut missing, mut errors) = (Vec::new(), Vec::new(), Vec::new());
    for (app, r) in found {
        match r {
            Ok(Some(latest)) => offers.push(Offer { app, installed: installed(app.name), latest }),
            Ok(None) => missing.push(app.name),
            Err(e) => errors.push(e),
        }
    }
    let note = if let Some(e) = errors.first() {
        Some(e.clone())
    } else if !missing.is_empty() {
        let (is, it) = if missing.len() == 1 { ("isn't", "it has") } else { ("aren't", "they have") };
        Some(format!("{} {is} listed: {it} no release yet.", missing.join(" and ")))
    } else {
        None
    };
    (offers, note)
}

fn programs() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join("Programs")
}

/// installs (or with `run` false, only downloads and checks) the ticked offers one after another; one that fails
/// doesn't stop the others
struct Install {
    offers: Vec<Offer>,
    run: bool,
}

impl Install {
    fn one(&self, o: &Offer, progress: &dyn Fn(&str, f32)) -> Result<(), String> {
        let App { name, repo, .. } = o.app;
        let v = &o.latest;
        let what = format!("Downloading {name} {v}");
        let setup = net::download(repo, name, v, AGENT, &|f| progress(&what, f * 0.8))?;
        if !self.run {
            return Ok(());
        }
        progress(&format!("Installing {name} {v}"), 0.8);
        let dir = std::env::temp_dir().join(format!("dv-setup-{}", std::process::id()));
        let exe = dir.join(net::setup_name(name, v));
        std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(&exe, &setup))
            .map_err(|e| format!("Cannot save its setup program: {e}"))?;
        let status = std::process::Command::new(&exe).arg("--quiet").status();
        let _ = std::fs::remove_dir_all(&dir);
        match status {
            Ok(s) if s.success() => Ok(()),
            Ok(s) => Err(format!("Its setup program failed ({s}).")),
            Err(e) => Err(format!("Cannot run its setup program: {e}")),
        }
    }
}

impl Work for Install {
    fn install(&self, ticked: &[bool], step: &dyn Fn(&str, f32)) -> Result<PathBuf, String> {
        let picked: Vec<&Offer> = self.offers.iter().zip(ticked).filter(|(_, &t)| t).map(|(o, _)| o).collect();
        let n = picked.len().max(1) as f32;
        let (mut done, mut failed) = (Vec::new(), Vec::new());
        for (k, o) in picked.iter().enumerate() {
            match self.one(o, &|what, f| step(what, (k as f32 + f) / n)) {
                Ok(()) => done.push(format!("{} {}", o.app.name, o.latest)),
                Err(e) => failed.push(format!("{} {}: {e}", o.app.name, o.latest)),
            }
        }
        step("Done", 1.0);
        if failed.is_empty() {
            return Ok(programs());
        }
        let mut text = failed.join("\n\n");
        if !done.is_empty() {
            text = format!("Installed: {}.\n\n{text}", done.join(", "));
        }
        Err(text)
    }

    fn launch(&self, _: &Path) {}
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x.eq_ignore_ascii_case(a));
    let (offers, note) = offers();
    let ticked: Vec<bool> = offers.iter().map(Offer::wanted).collect();
    if has("--quiet") || has("/S") {
        let r = Install { offers, run: true }.install(&ticked, &|_, _| {});
        std::process::exit(r.is_err() as i32);
    }
    let choices = offers.iter().zip(&ticked).map(|(o, &on)| Choice { label: o.label(), on, needs: None }).collect();
    let product = Product {
        name: "dv apps",
        scope: Scope::Apps { note },
        about: "Native Windows apps, each from its latest release on GitHub",
        location: programs(),
        choices,
        done: "Each app is in the Start menu, and Apps & features removes it. They were installed with their usual \
               options; run an app's own setup to change them."
            .into(),
        launch: None,
        activate: std::env::var_os("DVSETUP_NOACTIVATE").is_none(),
    };
    dv_setup::run(product, Install { offers, run: !has("--preview") });
}
