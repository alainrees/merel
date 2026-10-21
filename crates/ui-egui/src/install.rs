//! "Bijwerken": downloads the installer of a newer release for this platform, checks it
//! against the release's signed checksum list, installs it over this copy and starts it again.
//!
//! Only the file for this platform is fetched, and its address is built from our own
//! repository, the checked version and a fixed file name; nothing in GitHub's reply is
//! followed. Nothing is opened, mounted or run before the signature on `SHA256SUMS.txt`
//! checks out under the key this build carries ([`SIGNING_KEY`]) and the download matches its
//! line. Whenever a step fails the person gets the reason and the release page instead.
//!
//! - macOS: the `.dmg` is mounted read-only, its `Merel.app` copied next to this one and the two
//!   swapped, so a half-copied app never takes the place of a working one.
//! - Windows: the installer runs silently for the same user; it closes Merel, installs and
//!   starts the new version.
//! - Linux: a new AppImage replaces the one that runs. Merel from a `.deb` is updated by the
//!   package manager, so there the release page is offered.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use merel_secrets::handover::{self, ENV, HandoverKey};

use crate::help::REPOSITORY;

/// The public half of the release key (base64); with `None` every update goes through the
/// release page.
pub const SIGNING_KEY: Option<&str> = Some("xB4YYlhBVyeCC57QWyg+M/0o8jiAfle01CDt9myAr/8=");

/// The largest installer Merel downloads.
const MAX_INSTALLER_BYTES: u64 = 400 * 1024 * 1024;
const MAX_LIST_BYTES: u64 = 64 * 1024;

/// How this copy of Merel was installed, and so how it is replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// `Merel.app` at this path.
    MacApp(PathBuf),
    /// Installed by the Windows installer into this folder.
    Windows(PathBuf),
    /// The AppImage at this path.
    AppImage(PathBuf),
}

impl Install {
    /// How this copy runs, if Merel can update it itself: not when it runs from `cargo`, from a
    /// `.deb` or from a disk image.
    pub fn detect() -> Option<Self> {
        let exe = std::env::current_exe().ok()?;
        if cfg!(target_os = "macos") {
            app_bundle(&exe).map(Install::MacApp)
        } else if cfg!(target_os = "windows") {
            let dir = exe.parent()?;
            dir.join("unins000.exe").is_file().then(|| Install::Windows(dir.to_path_buf()))
        } else {
            let image = PathBuf::from(std::env::var_os("APPIMAGE")?);
            (image.is_absolute() && image.is_file() && cfg!(target_arch = "x86_64")).then_some(Install::AppImage(image))
        }
    }

    /// The release file for this platform.
    pub fn asset(&self, version: &str) -> String {
        match self {
            Install::MacApp(_) => format!("Merel-{version}-macos.dmg"),
            Install::Windows(_) => format!("Merel-{version}-windows-setup.exe"),
            Install::AppImage(_) => format!("Merel-{version}-linux-x86_64.AppImage"),
        }
    }
}

/// Whether "Bijwerken" can do the whole update here.
pub fn available() -> bool {
    SIGNING_KEY.is_some() && Install::detect().is_some()
}

/// The `.app` folder an executable sits in (`…/Merel.app/Contents/MacOS/merel`).
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let bundle = exe.parent()?.parent()?.parent()?;
    let macos = exe.parent()?.file_name()? == "MacOS";
    let contents = exe.parent()?.parent()?.file_name()? == "Contents";
    let app = bundle.extension().is_some_and(|e| e == "app");
    (macos && contents && app).then(|| bundle.to_path_buf())
}

/// A file of release `version`, at our own address.
pub fn release_file(version: &str, name: &str) -> String {
    format!("{REPOSITORY}/releases/download/v{version}/{name}")
}

/// Where an update stands; the window shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Downloading {
        done: u64,
        total: Option<u64>,
    },
    Checking,
    Installing,
    /// Installed (or, on Windows, checked and ready to run); what starts the new version.
    Ready(Restart),
    /// What went wrong, in words for the person.
    Failed(String),
}

/// What "Opnieuw starten" does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Restart {
    /// Opens the app or AppImage at this path once this process has ended.
    Relaunch(PathBuf),
    /// Runs the Windows installer silently; it closes Merel and starts the new version.
    Installer(PathBuf),
}

impl Restart {
    /// Starts what replaces this process; the caller then closes the window. `handover` is the
    /// key to the secrets [`hand_over`] put aside, which the new version gets in its
    /// environment (macOS only).
    pub fn start(&self, handover: Option<&HandoverKey>) -> std::io::Result<()> {
        use std::process::Command;
        match self {
            Restart::Relaunch(path) => {
                // Waits for this process to end, so two copies never share the files. The path
                // is an argument and the key an environment variable, never part of the script.
                let open = match (cfg!(target_os = "macos"), handover.is_some()) {
                    (true, true) => format!("exec open -n --env \"{ENV}=${ENV}\" \"$1\""),
                    (true, false) => "exec open -n \"$1\"".to_string(),
                    _ => "exec \"$1\"".to_string(),
                };
                let script = format!("while kill -0 \"$0\" 2>/dev/null; do sleep 0.2; done; {open}");
                let mut command = Command::new("/bin/sh");
                command.arg("-c").arg(script).arg(std::process::id().to_string()).arg(path);
                match handover {
                    Some(key) => command.env(ENV, key.expose()),
                    None => command.env_remove(ENV),
                };
                command.spawn().map(drop)
            }
            Restart::Installer(path) => {
                Command::new(path).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS"]).spawn().map(drop)
            }
        }
    }
}

/// Where the secrets wait for the next version during an update.
pub fn handover_file() -> Option<PathBuf> {
    Some(directories::ProjectDirs::from("nl", "", "Merel")?.cache_dir().join("handover"))
}

/// On macOS, puts the secrets aside for the version about to start, so it can file them as its
/// own and macOS does not ask for the keychain password (see [`merel_secrets::handover`]).
/// Elsewhere the credential store does not ask, and nothing happens.
pub fn hand_over(engine: &merel_engine::Engine) -> Option<HandoverKey> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let path = handover_file()?;
    match handover::export(&**engine.secrets(), &engine.secret_names(), &path) {
        Ok(key) => key,
        Err(e) => {
            log::warn!("secrets not handed over, the new version will ask for them: {e}");
            None
        }
    }
}

/// Puts the secrets back when the new version did not start after all.
pub fn take_back(engine: &merel_engine::Engine, key: &HandoverKey) {
    if let Some(path) = handover_file()
        && let Err(e) = handover::import(&**engine.secrets(), key.expose(), &path)
    {
        log::warn!("secrets could not be taken back: {e}");
    }
}

/// At start, files the secrets the previous version handed over, before anything reads the
/// keychain. Returns how many came over.
pub fn receive_handover(store: &dyn merel_secrets::SecretStore) -> usize {
    // The variable stays in this process's environment; the key is worthless once the file
    // below is gone, which it is whatever happens next.
    let Some(key) = std::env::var_os(ENV) else { return 0 };
    let Some(path) = handover_file() else { return 0 };
    match handover::import(store, &key.to_string_lossy(), &path) {
        Ok(n) => n,
        Err(e) => {
            log::warn!("handed-over secrets not received, Merel asks for them again: {e}");
            0
        }
    }
}

/// A running update.
pub struct Job {
    pub version: String,
    pub stage: Arc<Mutex<Stage>>,
    cancel: Arc<AtomicBool>,
}

impl Job {
    /// Starts updating to `version` on its own thread; `ctx` is repainted as it goes.
    pub fn start(ctx: &egui::Context, install: Install, version: &str, key: &'static str) -> Self {
        let stage = Arc::new(Mutex::new(Stage::Downloading { done: 0, total: None }));
        let cancel = Arc::new(AtomicBool::new(false));
        let job = Job { version: version.to_owned(), stage: stage.clone(), cancel: cancel.clone() };
        let ctx = ctx.clone();
        let version = version.to_owned();
        std::thread::spawn(move || {
            let report = |s: Stage| {
                if let Ok(mut slot) = stage.lock() {
                    *slot = s;
                }
                ctx.request_repaint();
            };
            let end = match run(&install, &version, key, &cancel, &report) {
                Ok(restart) => Stage::Ready(restart),
                Err(_) if cancel.load(Ordering::Relaxed) => return,
                Err(reason) => {
                    log::warn!("update to {version} failed: {reason}");
                    Stage::Failed(reason)
                }
            };
            report(end);
        });
        job
    }

    /// A job that stands at `stage`, for tests and pictures of the window.
    pub fn at(version: &str, stage: Stage) -> Self {
        Job { version: version.to_owned(), stage: Arc::new(Mutex::new(stage)), cancel: Arc::new(AtomicBool::new(false)) }
    }

    pub fn stage(&self) -> Stage {
        self.stage.lock().map(|s| s.clone()).unwrap_or(Stage::Failed("Bijwerken is onderbroken.".into()))
    }

    /// Stops a download; nothing is installed after this.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// A folder of our own for the download, emptied first; readable by this user only.
fn work_dir() -> Result<PathBuf, String> {
    let dir = directories::ProjectDirs::from("nl", "", "Merel").ok_or("Er is geen map om de download in te zetten.")?.cache_dir().join("update");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("De downloadmap kon niet worden gemaakt: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

fn agent() -> ureq::Agent {
    ureq::Agent::new_with_config(
        ureq::config::Config::builder()
            .tls_config(crate::remote::tls())
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(20 * 60)))
            .https_only(true)
            // A release file is served from GitHub's download host after a redirect.
            .max_redirects(5)
            .user_agent("Merel")
            .build(),
    )
}

fn fetch_small(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    agent
        .get(url)
        .call()
        .map_err(|e| format!("Downloaden is niet gelukt ({e})."))?
        .body_mut()
        .with_config()
        .limit(MAX_LIST_BYTES)
        .read_to_vec()
        .map_err(|e| format!("Downloaden is niet gelukt ({e})."))
}

fn run(install: &Install, version: &str, key: &str, cancel: &AtomicBool, report: &dyn Fn(Stage)) -> Result<Restart, String> {
    let agent = agent();
    let name = install.asset(version);
    // The list first: without a good signature nothing else is downloaded.
    let sums = fetch_small(&agent, &release_file(version, merel_sign::SUMS))?;
    let signature = fetch_small(&agent, &release_file(version, merel_sign::SIGNATURE))
        .map_err(|_| "Deze versie heeft geen handtekening, dus Merel installeert hem niet zelf.".to_owned())?;
    let signature = String::from_utf8_lossy(&signature);
    merel_sign::verify(key, &sums, &signature).map_err(|_| "De handtekening van deze versie klopt niet. Er is niets geïnstalleerd.".to_owned())?;

    let dir = work_dir()?;
    let file = dir.join(&name);
    let hash = download(&agent, &release_file(version, &name), &file, cancel, report)?;
    report(Stage::Checking);
    merel_sign::check(key, &sums, &signature, &name, &hash).map_err(|_| {
        let _ = std::fs::remove_file(&file);
        "De download komt niet overeen met de ondertekende versie. Er is niets geïnstalleerd.".to_owned()
    })?;
    if cancel.load(Ordering::Relaxed) {
        return Err("Geannuleerd.".into());
    }
    report(Stage::Installing);
    let restart = match install {
        Install::MacApp(app) => install_mac(&file, app, &dir).map(|()| Restart::Relaunch(app.clone())),
        Install::AppImage(image) => replace_file(&file, image).map(|()| Restart::Relaunch(image.clone())),
        Install::Windows(_) => return Ok(Restart::Installer(file)),
    };
    let _ = std::fs::remove_dir_all(&dir);
    restart
}

/// Downloads `url` to `to`, reporting progress; returns the SHA-256 of what came in.
fn download(agent: &ureq::Agent, url: &str, to: &Path, cancel: &AtomicBool, report: &dyn Fn(Stage)) -> Result<String, String> {
    let failed = |e: &dyn std::fmt::Display| format!("Downloaden is niet gelukt ({e}).");
    let mut response = agent.get(url).call().map_err(|e| failed(&e))?;
    let total = response.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok());
    if total.is_some_and(|t| t > MAX_INSTALLER_BYTES) {
        return Err("Het installatiebestand is groter dan verwacht. Er is niets geïnstalleerd.".into());
    }
    let mut body = response.body_mut().with_config().limit(MAX_INSTALLER_BYTES).reader();
    let mut out = std::fs::File::create(to).map_err(|e| failed(&e))?;
    let mut hasher = merel_sign::Hasher::default();
    let mut buffer = vec![0; 64 * 1024];
    let mut done = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Geannuleerd.".into());
        }
        let n = body.read(&mut buffer).map_err(|e| failed(&e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        out.write_all(&buffer[..n]).map_err(|e| failed(&e))?;
        done += n as u64;
        report(Stage::Downloading { done, total });
    }
    out.sync_all().map_err(|e| failed(&e))?;
    Ok(hasher.finish())
}

/// Puts the new AppImage in the old one's place in one rename.
fn replace_file(new: &Path, target: &Path) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("Installeren is niet gelukt ({e}).");
    let dir = target.parent().ok_or("Installeren is niet gelukt.")?;
    let staging = dir.join(".Merel-update.AppImage");
    std::fs::copy(new, &staging).map_err(failed)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o755)).map_err(failed)?;
    }
    std::fs::rename(&staging, target).map_err(|e| {
        let _ = std::fs::remove_file(&staging);
        failed(e)
    })
}

/// Copies `Merel.app` out of the disk image next to `app`, then swaps the two.
fn install_mac(dmg: &Path, app: &Path, dir: &Path) -> Result<(), String> {
    let mount = dir.join("mnt");
    std::fs::create_dir_all(&mount).map_err(|e| format!("Installeren is niet gelukt ({e})."))?;
    command(
        "hdiutil",
        &[
            "attach".as_ref(),
            "-nobrowse".as_ref(),
            "-readonly".as_ref(),
            "-noautoopen".as_ref(),
            "-mountpoint".as_ref(),
            mount.as_os_str(),
            dmg.as_os_str(),
        ],
    )?;
    let copied = (|| {
        let parent = app.parent().ok_or("Installeren is niet gelukt.")?;
        let staging = parent.join(".Merel-update.app");
        let _ = std::fs::remove_dir_all(&staging);
        // ditto keeps the app's signature and attributes intact.
        command("ditto", &[mount.join("Merel.app").as_os_str(), staging.as_os_str()])?;
        swap(&staging, app, &parent.join(".Merel-old.app"))
    })();
    let _ = command("hdiutil", &["detach".as_ref(), mount.as_os_str(), "-quiet".as_ref()]);
    copied
}

/// Moves `new` into `target`'s place, keeping the old one until the new one is there.
fn swap(new: &Path, target: &Path, old: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(old);
    std::fs::rename(target, old).map_err(|e| format!("Merel kon zichzelf niet vervangen ({e}). Staat Merel in de map Programma's?"))?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(old, target);
        return Err(format!("Merel kon zichzelf niet vervangen ({e})."));
    }
    let _ = std::fs::remove_dir_all(old);
    Ok(())
}

fn command(program: &str, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let status = std::process::Command::new(program).args(args).status().map_err(|e| format!("Installeren is niet gelukt ({program}: {e})."))?;
    if status.success() { Ok(()) } else { Err(format!("Installeren is niet gelukt ({program} gaf {status}).")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_for_each_platform_comes_from_our_own_release() {
        let mac = Install::MacApp("/Applications/Merel.app".into());
        assert_eq!(mac.asset("0.2.0"), "Merel-0.2.0-macos.dmg");
        assert_eq!(Install::Windows("C:/Merel".into()).asset("0.2.0"), "Merel-0.2.0-windows-setup.exe");
        assert_eq!(Install::AppImage("/home/a/Merel.AppImage".into()).asset("0.2.0"), "Merel-0.2.0-linux-x86_64.AppImage");
        assert_eq!(release_file("0.2.0", "SHA256SUMS.txt"), "https://github.com/alainrees/merel/releases/download/v0.2.0/SHA256SUMS.txt");
    }

    #[test]
    fn only_an_app_bundle_is_replaced_on_macos() {
        assert_eq!(app_bundle(Path::new("/Applications/Merel.app/Contents/MacOS/merel")), Some(PathBuf::from("/Applications/Merel.app")));
        assert_eq!(app_bundle(Path::new("/Users/a/merel/target/debug/merel")), None);
        assert_eq!(app_bundle(Path::new("/Applications/Merel/Contents/MacOS/merel")), None);
    }

    #[test]
    fn a_failed_swap_puts_the_old_app_back() {
        let dir = std::env::temp_dir().join(format!("merel-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Merel.app")).unwrap();
        std::fs::write(dir.join("Merel.app/old"), "1").unwrap();
        // No new app: the old one must stay where it was.
        assert!(swap(&dir.join("missing.app"), &dir.join("Merel.app"), &dir.join(".Merel-old.app")).is_err());
        assert!(dir.join("Merel.app/old").is_file());
        std::fs::create_dir_all(dir.join(".Merel-update.app")).unwrap();
        std::fs::write(dir.join(".Merel-update.app/new"), "2").unwrap();
        swap(&dir.join(".Merel-update.app"), &dir.join("Merel.app"), &dir.join(".Merel-old.app")).unwrap();
        assert!(dir.join("Merel.app/new").is_file());
        assert!(!dir.join(".Merel-old.app").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_new_appimage_takes_the_old_ones_place_and_runs() {
        let dir = std::env::temp_dir().join(format!("merel-appimage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Merel.AppImage"), "old").unwrap();
        std::fs::write(dir.join("download"), "new").unwrap();
        replace_file(&dir.join("download"), &dir.join("Merel.AppImage")).unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("Merel.AppImage")).unwrap(), "new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(dir.join("Merel.AppImage")).unwrap().permissions().mode() & 0o777, 0o755);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
