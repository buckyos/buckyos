//! Online items against an upstream served from this process. The manifest only takes `https`; in this crate's tests it
//! also takes the loopback address (`online_url_allowed`).

use super::*;
use std::collections::HashMap;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
enum Serve {
    Bytes(Vec<u8>),
    Status(u16),
    /// The headers and half the body, then silence.
    Stall(Vec<u8>),
}

struct Upstream {
    port: u16,
    routes: Arc<Mutex<HashMap<String, Serve>>>,
    hits: Arc<Mutex<HashMap<String, usize>>>,
}

fn answer(mut stream: TcpStream, routes: &Mutex<HashMap<String, Serve>>, hits: &Mutex<HashMap<String, usize>>) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
    }
    let path = String::from_utf8_lossy(&request).split_whitespace().nth(1).unwrap_or("/").to_string();
    *hits.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
    let what = routes.lock().unwrap().get(&path).cloned().unwrap_or(Serve::Status(404));
    let head = |status: &str, length: usize| format!("HTTP/1.1 {status}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
    let _ = match what {
        Serve::Bytes(body) => stream.write_all(head("200 OK", body.len()).as_bytes()).and_then(|_| stream.write_all(&body)),
        Serve::Status(code) => stream.write_all(head(&format!("{code} No"), 0).as_bytes()),
        Serve::Stall(body) => {
            let sent = stream.write_all(head("200 OK", body.len()).as_bytes()).and_then(|_| stream.write_all(&body[..body.len() / 2])).and_then(|_| stream.flush());
            std::thread::sleep(Duration::from_secs(3));
            sent
        }
    };
}

impl Upstream {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let routes = Arc::new(Mutex::new(HashMap::new()));
        let hits = Arc::new(Mutex::new(HashMap::new()));
        let (shared_routes, shared_hits) = (routes.clone(), hits.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (routes, hits) = (shared_routes.clone(), shared_hits.clone());
                std::thread::spawn(move || answer(stream, &routes, &hits));
            }
        });
        Self { port, routes, hits }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }

    fn serve(&self, path: &str, what: Serve) {
        self.routes.lock().unwrap().insert(path.to_string(), what);
    }

    fn hits(&self, path: &str) -> usize {
        *self.hits.lock().unwrap().get(path).unwrap_or(&0)
    }
}

const LINK: u32 = 0o120000;
const DIR: u32 = 0o040000;

/// `(name, unix mode, content)`: a mode of `DIR` is a directory, of `LINK` a symlink to `content`.
fn zip_bytes(entries: &[(&str, u32, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, mode, content) in entries {
        let options = zip::write::SimpleFileOptions::default().unix_permissions(mode & 0o7777);
        match mode & 0o170000 {
            DIR => writer.add_directory(*name, options).unwrap(),
            LINK => writer.add_symlink(*name, *content, options).unwrap(),
            _ => {
                writer.start_file(*name, options).unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
        }
    }
    writer.finish().unwrap().into_inner()
}

fn tar_gz_bytes(entries: &[(&str, u32, &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
    for (name, mode, content) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(*mode);
        builder.append_data(&mut header, name, content.as_bytes()).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn item(id: &str, version: &str, url: String, archive: &[u8], format: &str) -> ExtToolsOnlineItem {
    let source = ExtToolsOnlineSource { url, sha256: sha256_hex(archive), size_bytes: archive.len() as u64, format: format.to_string(), into: None, rename: vec![], overlay: None };
    ExtToolsOnlineItem {
        id: id.to_string(),
        version: version.to_string(),
        desc: String::new(),
        license: String::new(),
        auto: true,
        platforms: BTreeMap::from([("linux-x86_64".to_string(), source)]),
    }
}

/// An archive whose `run` says which version it is.
fn runnable(version: &str, also: &str) -> Vec<u8> {
    zip_bytes(&[("run", 0o755, &format!("#!/bin/sh\necho item {version}\n")), (also, 0o644, "")])
}

/// How a launcher finds the tools root: one level up where it is installed, three from the payload being checked in
/// `staging/<name>/bin`.
const FIND_ROOT: &str = "here=\"$(cd \"${0%/*}\" && pwd -P)\"\ncase \"$here\" in */staging/*/bin) root=\"$here/../../..\" ;; *) root=\"$here/..\" ;; esac\n";

/// A payload with `<id>-cmd` and, with `commands`, a `run-<item>` for each online item that needs it: its launcher runs
/// the item's own `run`. `overlay` are files of the payload, scripts among them executable.
fn package(dir: &Path, id: &str, version: &str, online: Vec<ExtToolsOnlineItem>, commands: bool, overlay: &[(&str, &str)]) -> PathBuf {
    package_running(dir, id, version, online, commands, overlay, "exec \"$root/online/$item/run\" \"$@\"\n")
}

/// `run` is what the launcher of `run-<item>` does once it knows `$root` and `$item`.
fn package_running(dir: &Path, id: &str, version: &str, online: Vec<ExtToolsOnlineItem>, commands: bool, overlay: &[(&str, &str)], run: &str) -> PathBuf {
    let command = format!("{id}-cmd");
    let mut m = manifest(id, version, &[command.as_str()], &[]);
    for item in online.iter().filter(|_| commands) {
        m.provides.push(ExtToolsCommand {
            command: format!("run-{}", item.id),
            version_probe: format!("run-{} --version", item.id),
            smoke: Some(format!("smoke/{}.sh", item.id)),
            needs: vec![item.id.clone()],
        });
    }
    m.online = online;
    let tree = dir.join(format!("tree-{id}-{version}"));
    let _ = fs::remove_dir_all(&tree);
    let write = |path: &str, content: &str, mode: u32| {
        let file = tree.join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, content).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(mode)).unwrap();
    };
    write(EXT_TOOLS_MANIFEST_FILE, &serde_json::to_string(&m).unwrap(), 0o644);
    write(&format!("store/{id}/VERSION"), version, 0o644);
    write(&format!("bin/{id}-cmd"), &format!("#!/bin/sh\necho {id}-cmd {version}\n"), 0o755);
    write("smoke/all.sh", &format!("#!/bin/sh\n{id}-cmd | grep -q {version}\n"), 0o644);
    for item in m.online.iter().filter(|_| commands) {
        write(&format!("bin/run-{}", item.id), &format!("#!/bin/sh\n{FIND_ROOT}item={}\n{run}", item.id), 0o755);
        write(&format!("smoke/{}.sh", item.id), &format!("#!/bin/sh\nrun-{} --version | grep -q item\n", item.id), 0o644);
    }
    for (path, content) in overlay {
        write(path, content, if content.starts_with("#!") { 0o755 } else { 0o644 });
    }
    let tar_path = dir.join(format!("{id}-{version}.tar.gz"));
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(fs::File::create(&tar_path).unwrap(), flate2::Compression::fast()));
    builder.append_dir_all(".", &tree).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    tar_path
}

fn download(root: &ToolsRoot, item: &str, request_ms: u64) -> (ExtToolsResult<ExtToolsOnlineRecord>, u64) {
    let job = ExtToolsOnlineJob::new(request_ms);
    let limits = ExtToolsOnlineLimits { connect: Duration::from_secs(2), stall: Duration::from_millis(500), total: Duration::from_secs(30) };
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let result = runtime.block_on(ext_tools_online_install(root.path(), item, &limits, &job));
    (result, job.downloaded_bytes.load(Ordering::Relaxed))
}

fn state(root: &ToolsRoot, item: &str) -> ExtToolsOnlineState {
    root.online()[item].status.state
}

fn staging_is_empty(root: &ToolsRoot) -> bool {
    fs::read_dir(root.path().join("staging")).unwrap().count() == 0
}

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

#[test]
fn an_online_item_is_downloaded_unpacked_and_laid_out_and_its_command_waits_until_then() {
    let scratch = Scratch::new("online-ready");
    let upstream = Upstream::start();
    let archive = zip_bytes(&[
        ("chrome-linux64/", DIR | 0o700, ""),
        ("chrome-linux64/chrome", 0o4700, "#!/bin/sh\necho item 159.0\n"),
        ("chrome-linux64/resources.pak", 0o600, "data"),
        ("chrome-linux64/latest", LINK, "chrome"),
    ]);
    upstream.serve("/chrome.zip", Serve::Bytes(archive.clone()));
    let mut browser = item("browser", "159.0", upstream.url("/chrome.zip"), &archive, "zip");
    let source = browser.platforms.get_mut("linux-x86_64").unwrap();
    source.into = Some("rev-1".to_string());
    source.rename = vec![("rev-1/chrome-linux64/chrome".to_string(), "rev-1/chrome-linux64/chrome-bin".to_string())];
    source.overlay = Some("online/browser".to_string());
    let overlay = [
        ("online/browser/run", "#!/bin/sh\nexec \"${0%/*}/rev-1/chrome-linux64/chrome\" \"$@\"\n"),
        ("online/browser/rev-1/chrome-linux64/chrome", "#!/bin/sh\nexec \"${0%/*}/chrome-bin\" \"$@\"\n"),
        ("online/browser/rev-1/INSTALLATION_COMPLETE", ""),
    ];
    let root = ToolsRoot::new(scratch.0.join("tools"));
    let tar = package(&scratch.0, "ext-a", "1.0.0", vec![browser], true, &overlay);

    let install = root.install(&apply(&tar, "ext-a"), &mut |_| {}).unwrap();
    assert_eq!(install.smoke, ExtToolsSmokeResult::Passed, "the smoke of a command that waits is not run");
    assert_eq!(install.waiting, BTreeMap::from([("run-browser".to_string(), vec!["browser".to_string()])]));
    assert_eq!(install.commands["run-browser"], "");
    assert_eq!(install.commands["ext-a-cmd"], "ext-a-cmd 1.0.0");
    assert_eq!(root.install_record("ext-a").unwrap().waiting, install.waiting);
    root.health("ext-a").unwrap();
    root.check("ext-a").unwrap();
    let entry = &root.online()["browser"];
    assert_eq!((entry.package.as_str(), entry.status.state, entry.auto), ("ext-a", ExtToolsOnlineState::Missing, true));
    assert_eq!(entry.status.size_bytes, archive.len() as u64);
    assert!(root.online_record("browser").is_none(), "no record: never downloaded");

    let (record, downloaded) = download(&root, "browser", 0);
    let record = record.unwrap();
    assert_eq!(downloaded, archive.len() as u64);
    assert_eq!(record, root.online_record("browser").unwrap());
    assert_eq!(serde_json::to_value(&record).unwrap(), serde_json::json!({
        "schema": "buckyos.ext_tools.online/1", "id": "browser", "version": "159.0", "package": "ext-a", "platform": "linux-x86_64",
        "sha256": sha256_hex(&archive), "state": "ready", "size_bytes": archive.len(), "downloaded_bytes": archive.len(),
        "error": null, "updated_at_ms": record.updated_at_ms, "request_ms": 0,
    }));
    assert_eq!(upstream.hits("/chrome.zip"), 1);
    assert!(staging_is_empty(&root));

    // A worker and the system's page renderer are other accounts: whatever the archive and the umask say, they get in.
    let place = root.path().join("online/browser/rev-1");
    assert_eq!(mode(&place.join("chrome-linux64/chrome-bin")), 0o755, "what was executable stays so, for everyone and without the set-id bit");
    assert_eq!(mode(&place.join("chrome-linux64/resources.pak")), 0o644);
    for dir in ["online", "online/browser", "online/browser/rev-1", "online/browser/rev-1/chrome-linux64"] {
        assert_eq!(mode(&root.path().join(dir)), 0o755, "{dir}");
    }
    assert_eq!(mode(&root.path().join("online/browser.json")), 0o644);
    assert_eq!(mode(&root.path().join("online/browser/run")), 0o755);
    assert_eq!(mode(&place.join("INSTALLATION_COMPLETE")), 0o644);
    assert_eq!(fs::read_link(place.join("chrome-linux64/latest")).unwrap(), PathBuf::from("chrome"));
    assert!(fs::read_to_string(place.join("chrome-linux64/chrome")).unwrap().contains("chrome-bin"), "the overlay replaces what was renamed away");
    assert!(place.join("INSTALLATION_COMPLETE").is_file());
    assert_eq!(run_command(root.path(), "run-browser"), "item 159.0");

    let install = root.install_record("ext-a").unwrap();
    assert!(install.waiting.is_empty());
    assert_eq!(install.commands["run-browser"], "item 159.0", "probed once its item arrived");
    root.check("ext-a").unwrap();
    fs::remove_file(place.join("chrome-linux64/chrome-bin")).unwrap();
    let broken = root.check("ext-a").unwrap_err();
    assert!(broken.contains("smoke/browser.sh"), "{broken}");
}

#[test]
fn a_download_that_fails_leaves_nothing_and_stays_failed_until_asked_again() {
    let scratch = Scratch::new("online-failed");
    let upstream = Upstream::start();
    let archive = runnable("1", "only-1");
    let mut corrupt = archive.clone();
    *corrupt.last_mut().unwrap() ^= 0xff;
    let closed = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let root = ToolsRoot::new(scratch.0.join("tools"));
    let online = vec![item("x", "1", upstream.url("/x.zip"), &archive, "zip"), item("gone", "1", format!("http://127.0.0.1:{closed}/gone.zip"), &archive, "zip")];
    let tar = package(&scratch.0, "ext-a", "1.0.0", online, true, &[]);
    root.install(&apply(&tar, "ext-a"), &mut |_| {}).unwrap();

    let failed = |error: String, about: &str, request_ms: u64| {
        assert!(error.contains(about), "{error}");
        let record = root.online_record("x").unwrap();
        assert_eq!((record.state, record.error.as_deref(), record.request_ms), (ExtToolsOnlineState::Failed, Some(error.as_str()), request_ms));
        assert_eq!(state(&root, "x"), ExtToolsOnlineState::Failed, "a failure is not `missing`: nothing downloads it again by itself");
        assert!(!root.path().join("online/x").exists(), "no half of a directory");
        assert!(staging_is_empty(&root));
        assert_eq!(root.install_record("ext-a").unwrap().waiting["run-x"], vec!["x".to_string()]);
        root.health("ext-a").unwrap();
    };

    upstream.serve("/x.zip", Serve::Bytes(corrupt));
    failed(download(&root, "x", 0).0.unwrap_err(), "sha256 is", 0);
    assert_eq!(upstream.hits("/x.zip"), 1, "one attempt is one request");

    upstream.serve("/x.zip", Serve::Status(404));
    failed(download(&root, "x", 3).0.unwrap_err(), "HTTP 404", 3);

    upstream.serve("/x.zip", Serve::Stall(archive.clone()));
    failed(download(&root, "x", 3).0.unwrap_err(), "nothing received", 3);

    let mut longer = archive.clone();
    longer.extend_from_slice(b"more");
    upstream.serve("/x.zip", Serve::Bytes(longer));
    failed(download(&root, "x", 3).0.unwrap_err(), "more than", 3);

    upstream.serve("/x.zip", Serve::Bytes(zip_bytes(&[("run", 0o755, "x")])));
    failed(download(&root, "x", 3).0.unwrap_err(), "sha256 is", 3);

    let refused = download(&root, "gone", 0).0.unwrap_err();
    assert!(refused.contains("gone.zip"), "{refused}");
    assert_eq!(state(&root, "gone"), ExtToolsOnlineState::Failed);

    upstream.serve("/x.zip", Serve::Bytes(archive.clone()));
    let record = download(&root, "x", 9).0.unwrap();
    assert_eq!((record.state, record.request_ms, record.error), (ExtToolsOnlineState::Ready, 9, None));
    assert_eq!(run_command(root.path(), "run-x"), "item 1");
    assert_eq!(upstream.hits("/x.zip"), 6);
    assert!(root.install_record("ext-a").unwrap().waiting.contains_key("run-gone") && !root.install_record("ext-a").unwrap().waiting.contains_key("run-x"));

    // Asked for again while it is there: what is in place stays in use whatever becomes of the new download.
    upstream.serve("/x.zip", Serve::Status(404));
    assert!(download(&root, "x", 12).0.unwrap_err().contains("HTTP 404"));
    let record = root.online_record("x").unwrap();
    assert_eq!((record.state, record.error.clone(), record.request_ms), (ExtToolsOnlineState::Ready, None, 12), "whoever uses the item sees it ready");
    let again = record.next.unwrap();
    assert_eq!((again.state, again.version.as_str()), (ExtToolsOnlineState::Failed, "1"));
    let status = root.online()["x"].status.clone();
    assert_eq!(status.state, ExtToolsOnlineState::Ready);
    assert!(status.error.unwrap().contains("HTTP 404"), "and whoever asked sees why it was not replaced");
    assert_eq!(run_command(root.path(), "run-x"), "item 1");
    assert!(!root.install_record("ext-a").unwrap().waiting.contains_key("run-x"));
    assert!(staging_is_empty(&root));

    upstream.serve("/x.zip", Serve::Bytes(archive));
    let record = download(&root, "x", 15).0.unwrap();
    assert_eq!((record.state, record.request_ms, record.next.clone()), (ExtToolsOnlineState::Ready, 15, None));
    assert_eq!(root.online()["x"].status.error, None);
}

#[test]
fn an_upgrade_keeps_an_item_of_the_same_version_and_replaces_another_whole() {
    let scratch = Scratch::new("online-upgrade");
    let upstream = Upstream::start();
    let (one, two) = (runnable("1", "only-1"), runnable("2", "only-2"));
    upstream.serve("/1.zip", Serve::Bytes(one.clone()));
    upstream.serve("/2.zip", Serve::Bytes(two.clone()));
    let root = ToolsRoot::new(scratch.0.join("tools"));
    // Every package lays a note of its own version over the item.
    let pack = |version: &str, mut online: ExtToolsOnlineItem, run: &str| {
        online.platforms.get_mut("linux-x86_64").unwrap().overlay = Some("online/x".to_string());
        package_running(&scratch.0, "ext-a", version, vec![online], true, &[("online/x/note", version)], run)
    };
    let install = |version: &str, online: ExtToolsOnlineItem| {
        let tar = pack(version, online, "exec \"$root/online/$item/run\" \"$@\"\n");
        root.install(&apply(&tar, "ext-a"), &mut |_| {}).unwrap()
    };
    let place = root.path().join("online/x");
    let note = || fs::read_to_string(place.join("note")).unwrap();

    install("1.0.0", item("x", "1", upstream.url("/1.zip"), &one, "zip"));
    download(&root, "x", 0).0.unwrap();
    assert_eq!(note(), "1.0.0");
    let downloaded = root.online_record("x").unwrap();

    let same = install("1.1.0", item("x", "1", upstream.url("/1.zip"), &one, "zip"));
    assert_eq!(state(&root, "x"), ExtToolsOnlineState::Ready, "the same version is not downloaded again");
    assert!(same.waiting.is_empty());
    assert_eq!(same.commands["run-x"], "item 1");
    assert_eq!(upstream.hits("/1.zip"), 1);
    assert_eq!(note(), "1.1.0", "but the package's overlay is its own, and comes with it");
    assert_eq!(mode(&place.join("note")), 0o644);
    assert_eq!(root.online_record("x").unwrap(), downloaded, "nothing was downloaded for that");

    // The item is here, so the smoke of the command that needs it is part of checking the next package.
    let broken = pack("1.2.0", item("x", "1", upstream.url("/1.zip"), &one, "zip"), "exit 3\n");
    let refused = root.install(&apply(&broken, "ext-a"), &mut |_| {}).unwrap_err();
    assert!(refused.contains("smoke/x.sh"), "{refused}");
    assert_eq!((root.install_record("ext-a").unwrap().version.as_str(), note().as_str()), ("1.1.0", "1.1.0"), "what is active stays as it was");

    let next = install("2.0.0", item("x", "2", upstream.url("/2.zip"), &two, "zip"));
    assert_eq!(state(&root, "x"), ExtToolsOnlineState::Missing, "another version: to be downloaded");
    assert_eq!(next.waiting["run-x"], vec!["x".to_string()]);
    assert_eq!(next.commands["run-x"], "");
    assert!(place.join("only-1").is_file(), "the old one stays until the new one is here");
    assert_eq!(note(), "1.1.0", "and so does what lies over it");

    // While the new one downloads, the one in place is what the record speaks of: its users keep it until the swap.
    let began = root.online_begin("x", 0).unwrap();
    assert_eq!((began.status.state, began.status.version.as_str()), (ExtToolsOnlineState::Downloading, "2"));
    root.online_progress("x", 5);
    let record = root.online_record("x").unwrap();
    assert_eq!((record.state, record.version.as_str(), record.downloaded_bytes), (ExtToolsOnlineState::Ready, "1", one.len() as u64));
    let next = record.next.unwrap();
    assert_eq!((next.state, next.version.as_str(), next.downloaded_bytes, next.size_bytes), (ExtToolsOnlineState::Downloading, "2", 5, two.len() as u64));
    assert_eq!(root.online()["x"].status.downloaded_bytes, 5);
    assert!(place.join("only-1").is_file());
    let archive = scratch.0.join("two.zip");
    fs::write(&archive, &two).unwrap();
    let record = root.online_apply("x", &archive, 0).unwrap();
    assert_eq!((record.state, record.version.as_str(), record.next.clone()), (ExtToolsOnlineState::Ready, "2", None));
    assert!(place.join("only-2").is_file() && !place.join("only-1").exists(), "replaced whole");
    assert_eq!(note(), "2.0.0");
    assert_eq!(root.install_record("ext-a").unwrap().commands["run-x"], "item 2");
    assert!(staging_is_empty(&root));

    install("3.0.0", item("x", "3", upstream.url("/3.zip"), &one, "zip"));
    assert!(download(&root, "x", 0).0.unwrap_err().contains("HTTP 404"));
    assert_eq!(state(&root, "x"), ExtToolsOnlineState::Failed);
    let record = root.online_record("x").unwrap();
    assert_eq!((record.state, record.version.as_str(), record.next), (ExtToolsOnlineState::Failed, "3", None), "the version the package needs is not here, and the record says so");
    assert!(place.join("only-2").is_file(), "a failed download leaves what was there");
}

#[test]
fn an_item_leaves_with_the_last_package_that_declares_it() {
    let scratch = Scratch::new("online-remove");
    let upstream = Upstream::start();
    let shared = runnable("1", "only-1");
    let packed = tar_gz_bytes(&[("run", 0o700, "#!/bin/sh\necho item tar\n"), ("data/readme", 0o600, "x")]);
    upstream.serve("/x.zip", Serve::Bytes(shared.clone()));
    upstream.serve("/y.tar.gz", Serve::Bytes(packed.clone()));
    let root = ToolsRoot::new(scratch.0.join("tools"));
    let x = || item("x", "1", upstream.url("/x.zip"), &shared, "zip");
    let a = package(&scratch.0, "ext-a", "1.0.0", vec![x()], true, &[]);
    let b = package(&scratch.0, "ext-b", "1.0.0", vec![x(), item("y", "1", upstream.url("/y.tar.gz"), &packed, "tar.gz")], false, &[]);
    root.install(&apply(&a, "ext-a"), &mut |_| {}).unwrap();
    root.install(&apply(&b, "ext-b"), &mut |_| {}).unwrap();
    assert_eq!(root.online()["x"].package, "ext-a", "one item id is one thing on a node");
    download(&root, "x", 0).0.unwrap();
    download(&root, "y", 0).0.unwrap();
    assert_eq!(mode(&root.path().join("online/y/run")), 0o755);
    assert_eq!(mode(&root.path().join("online/y/data/readme")), 0o644);
    assert_eq!((mode(&root.path().join("online/y")), mode(&root.path().join("online/y/data"))), (0o755, 0o755), "directories the archive only implies too");

    root.remove("ext-a").unwrap();
    assert_eq!((root.online()["x"].package.as_str(), state(&root, "x")), ("ext-b", ExtToolsOnlineState::Ready), "still declared");
    root.remove("ext-b").unwrap();
    assert!(root.online().is_empty());
    assert_eq!(fs::read_dir(root.path().join("online")).unwrap().count(), 0, "the items and their records go with the last package");
}

#[test]
fn a_download_for_a_version_the_package_dropped_is_not_put_in_place() {
    let scratch = Scratch::new("online-superseded");
    let upstream = Upstream::start();
    let (one, two) = (runnable("1", "only-1"), runnable("2", "only-2"));
    upstream.serve("/2.zip", Serve::Bytes(two.clone()));
    let root = ToolsRoot::new(scratch.0.join("tools"));
    let first = package(&scratch.0, "ext-a", "1.0.0", vec![item("x", "1", upstream.url("/1.zip"), &one, "zip")], true, &[]);
    root.install(&apply(&first, "ext-a"), &mut |_| {}).unwrap();
    let began = root.online_begin("x", 7).unwrap();
    assert_eq!((began.status.state, began.request_ms), (ExtToolsOnlineState::Downloading, 7));
    root.online_progress("x", 5);
    assert_eq!(root.online()["x"].status.downloaded_bytes, 5);

    let second = package(&scratch.0, "ext-a", "2.0.0", vec![item("x", "2", upstream.url("/2.zip"), &two, "zip")], true, &[]);
    root.install(&apply(&second, "ext-a"), &mut |_| {}).unwrap();
    let archive = scratch.0.join("one.zip");
    fs::write(&archive, &one).unwrap();
    assert!(root.online_apply("x", &archive, 7).unwrap_err().contains("no longer declares"));
    assert!(root.online_fail("x", "cut short", 7).unwrap_err().contains("no longer declares"));
    assert!(!root.path().join("online/x").exists());
    let entry = &root.online()["x"];
    assert_eq!((entry.status.state, entry.version.as_str(), entry.request_ms), (ExtToolsOnlineState::Missing, "2", 7), "the new version is still to be downloaded");

    let record = download(&root, "x", 0).0.unwrap();
    assert_eq!((record.version.as_str(), record.request_ms), ("2", 7), "a request once answered stays answered");
    assert!(root.online_begin("nope", 0).unwrap_err().contains("no installed package declares"));
}

#[test]
fn a_zip_cannot_reach_outside_its_directory() {
    let scratch = Scratch::new("online-zip");
    let unpack = |name: &str, entries: &[(&str, u32, &str)]| {
        let archive = scratch.0.join(format!("{name}.zip"));
        fs::write(&archive, zip_bytes(entries)).unwrap();
        ext_tools_unzip(&archive, &scratch.0.join(name))
    };
    fs::create_dir_all(scratch.0.join("outside")).unwrap();

    unpack("fine", &[("app/Versions/A/bin", 0o755, "x"), ("app/Versions/Current", LINK, "A"), ("app/bin", LINK, "Versions/Current/bin")]).unwrap();
    assert_eq!(fs::read_to_string(scratch.0.join("fine/app/bin")).unwrap(), "x", "an app bundle's links lead where they did");
    assert!(fs::symlink_metadata(scratch.0.join("fine/app/Versions/Current")).unwrap().file_type().is_symlink());

    assert!(unpack("dotdot", &[("../outside/evil", 0o644, "x")]).unwrap_err().contains("leaves the item"));
    let _ = unpack("absolute", &[(&format!("{}/outside/evil", scratch.0.display()), 0o644, "x")]);
    assert!(unpack("up", &[("a/link", LINK, "../../outside")]).unwrap_err().contains("links outside"));
    assert!(unpack("abs-link", &[("link", LINK, "/etc")]).unwrap_err().contains("links outside"));
    assert!(unpack("behind", &[("here", LINK, "."), ("here/up", LINK, "..")]).unwrap_err().contains("behind a symlink"));
    assert!(unpack("through", &[("here", LINK, "."), ("up", LINK, "here/..")]).unwrap_err().contains("links outside"), "as the disk resolves it");
    assert!(!scratch.0.join("outside/evil").exists());
}

/// The font package as publish/exttool/ext-fonts declares it: every font from its upstream's release on GitHub, and the
/// directory its fonts.conf names in each, from where the package is active. By hand:
/// `cargo test -p buckyos-api --lib -- --ignored --nocapture the_fonts_come_from_their_upstreams`.
#[test]
#[ignore = "downloads about 110 MB from github.com"]
fn the_fonts_come_from_their_upstreams() {
    let scratch = Scratch::new("online-fonts");
    let root = ToolsRoot::new(scratch.0.join("tools"));
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../publish/exttool/ext-fonts");
    let tree = scratch.0.join("tree");
    fs::create_dir_all(tree.join("store/ext-fonts")).unwrap();
    fs::copy(source.join(EXT_TOOLS_MANIFEST_FILE), tree.join(EXT_TOOLS_MANIFEST_FILE)).unwrap();
    fs::copy(source.join("fonts.conf"), tree.join("store/ext-fonts/fonts.conf")).unwrap();
    let tar_path = scratch.0.join("ext-fonts.tar.gz");
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(fs::File::create(&tar_path).unwrap(), flate2::Compression::fast()));
    builder.append_dir_all(".", &tree).unwrap();
    builder.into_inner().unwrap().finish().unwrap();
    root.install(&apply(&tar_path, "ext-fonts"), &mut |_| {}).unwrap();

    let conf = fs::read_to_string(root.path().join("store/ext-fonts/fonts.conf")).unwrap();
    let fonts_dir = fs::canonicalize(root.path().join("store/ext-fonts")).unwrap();
    let dirs: Vec<&str> = conf.split("<dir prefix=\"relative\">").skip(1).filter_map(|rest| rest.split("</dir>").next()).collect();
    let items: Vec<String> = root.online().into_keys().collect();
    assert_eq!(dirs.len(), items.len());
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    for item in &items {
        let record = runtime.block_on(ext_tools_online_install(root.path(), item, &ExtToolsOnlineLimits::default(), &ExtToolsOnlineJob::new(0))).unwrap();
        assert_eq!(record.state, ExtToolsOnlineState::Ready, "{item}");
        let dir = dirs.iter().find(|dir| dir.contains(&format!("/online/{item}/"))).unwrap_or_else(|| panic!("{item}: not in fonts.conf"));
        let fonts = fs::canonicalize(fonts_dir.join(dir)).unwrap_or_else(|e| panic!("{item}: {dir}: {e}"));
        let ttf = fs::read_dir(&fonts).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|ext| ext == "ttf")).count();
        assert!(ttf > 0, "{item}: no TrueType font in {}", fonts.display());
        println!("{item} {}: {} bytes, {ttf} fonts in {dir}", record.version, record.size_bytes);
    }
    assert!(staging_is_empty(&root));
}

/// The real thing, as `ext-core` declares it: Chrome for Testing from Google's download site. By hand:
/// `cargo test -p buckyos-api --lib -- --ignored --nocapture chrome_for_testing`.
#[test]
#[ignore = "downloads about 200 MB from storage.googleapis.com"]
fn chrome_for_testing_comes_from_its_upstream() {
    let scratch = Scratch::new("online-chrome");
    let root = ToolsRoot::new(scratch.0.join("tools"));
    let url = "https://storage.googleapis.com/chrome-for-testing-public/159.0.8154.6/linux64/chrome-linux64.zip";
    let mut chromium = item("chromium", "159.0.8154.6", url.to_string(), &[], "zip");
    let source = chromium.platforms.get_mut("linux-x86_64").unwrap();
    source.sha256 = "370601106c1b5619aae5f06f7a3fdc60836486135934779461e2a208d777f596".to_string();
    source.size_bytes = 208_987_836;
    source.into = Some("chromium-1255".to_string());
    source.rename = vec![("chromium-1255/chrome-linux64/chrome".to_string(), "chromium-1255/chrome-linux64/chrome-bin".to_string())];
    source.overlay = Some("online/chromium".to_string());
    let size = source.size_bytes;
    let overlay = [
        ("online/chromium/run", "#!/bin/sh\nexec \"${0%/*}/chromium-1255/chrome-linux64/chrome\" \"$@\"\n"),
        ("online/chromium/chromium-1255/chrome-linux64/chrome", "#!/bin/sh\nexec \"${0%/*}/chrome-bin\" \"$@\"\n"),
        ("online/chromium/chromium-1255/INSTALLATION_COMPLETE", ""),
    ];
    let tar = package(&scratch.0, "ext-a", "1.0.0", vec![chromium], true, &overlay);
    root.install(&apply(&tar, "ext-a"), &mut |_| {}).unwrap();

    let job = Arc::new(ExtToolsOnlineJob::new(0));
    let started = Instant::now();
    let watched = job.clone();
    let fetched = std::thread::spawn(move || {
        while watched.downloaded_bytes.load(Ordering::Relaxed) < size && started.elapsed() < Duration::from_secs(3600) {
            std::thread::sleep(Duration::from_millis(20));
        }
        started.elapsed()
    });
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let record = runtime.block_on(ext_tools_online_install(root.path(), "chromium", &ExtToolsOnlineLimits::default(), &job)).unwrap();
    let (total, fetched) = (started.elapsed(), fetched.join().unwrap());
    assert_eq!((record.state, record.downloaded_bytes), (ExtToolsOnlineState::Ready, size));
    let place = root.path().join("online/chromium/chromium-1255");
    assert_eq!(mode(&place.join("chrome-linux64/chrome-bin")), 0o755);
    assert!(place.join("INSTALLATION_COMPLETE").is_file());
    assert!(staging_is_empty(&root));
    fn count(dir: &Path) -> usize {
        fs::read_dir(dir).unwrap().flatten().map(|e| if e.path().is_dir() { count(&e.path()) } else { 1 }).sum()
    }
    println!(
        "chrome for testing: {size} bytes fetched in {:.1}s; checked, unpacked ({} files) and placed in {:.1}s; `chrome-bin --version`: {:?}",
        fetched.as_secs_f64(),
        count(&place),
        (total - fetched).as_secs_f64(),
        root.install_record("ext-a").unwrap().commands["run-chromium"],
    );
}
