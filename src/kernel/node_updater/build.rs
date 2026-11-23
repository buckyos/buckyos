use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::Path,
};

#[derive(Deserialize)]
struct Registry {
    reader: Vec<Reader>,
    audited_sources: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reader {
    id: String,
    owner: String,
    rust: String,
    areas: Vec<String>,
    depends: Vec<String>,
    audit: Vec<String>,
    schema: String,
    library: String,
}

mod registry_paths;

fn main() {
    println!("cargo:rerun-if-changed=readers.toml");
    let reg: Registry = toml::from_str(&fs::read_to_string("readers.toml").unwrap()).unwrap();
    let cargo: toml::Table = toml::from_str(&fs::read_to_string("Cargo.toml").unwrap()).unwrap();
    // A clean workspace resolves the branch dependencies again. Report a new
    // dependency revision even when the format owners have no in-repo changes.
    println!("cargo:rerun-if-changed=../../Cargo.lock");
    let lock: toml::Table =
        toml::from_str(&fs::read_to_string("../../Cargo.lock").unwrap()).unwrap();
    let mut source_revisions = BTreeMap::new();
    for (repo, audited) in &reg.audited_sources {
        let url = format!("git+https://github.com/buckyos/{repo}.git");
        let revisions: BTreeSet<_> = lock["package"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|package| package.get("source")?.as_str())
            .filter_map(|source| source.split_once('#'))
            .filter(|(location, _)| location.split('?').next() == Some(url.as_str()))
            .map(|(_, revision)| revision)
            .collect();
        if revisions.len() != 1 || !revisions.contains(audited.as_str()) {
            println!("cargo:warning=Review Desktop reader formats for {repo}, audited {audited}, resolved git revisions {revisions:?}");
        }
        source_revisions.insert(repo, revisions);
    }
    let out = env::var_os("OUT_DIR").unwrap();
    fs::write(
        Path::new(&out).join("format_sources.json"),
        serde_json::to_vec(&source_revisions).unwrap(),
    )
    .unwrap();
    let mut schemas = std::collections::BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut owned: Vec<String> = Vec::new();
    let mut code =
        String::from("fn run_registered(ctx: &mut Context<'_>) -> Result<(), Failure> {\n");
    for r in reg.reader {
        assert!(
            !r.owner.is_empty()
                && !r.schema.is_empty()
                && !r.areas.is_empty()
                && !r.audit.is_empty()
        );
        assert!(
            r.depends.iter().all(|d| seen.contains(d)),
            "{} has unknown, cyclic or unordered dependencies",
            r.id
        );
        assert!(seen.insert(r.id.clone()), "duplicate reader {}", r.id);
        assert!(
            r.library == "node_updater"
                || cargo["dependencies"]
                    .as_table()
                    .unwrap()
                    .contains_key(&r.library),
            "unlinked reader library {}",
            r.library
        );
        schemas.insert(r.id.clone(), r.schema);
        for a in r.areas {
            assert!(
                !owned.iter().any(|b| registry_paths::overlaps(&a, b)),
                "duplicate ownership: {a}"
            );
            owned.push(a);
        }
        for p in r.audit {
            if let Some((repo, source)) = p.split_once(':') {
                assert!(
                    reg.audited_sources.contains_key(repo),
                    "undeclared format source {repo}"
                );
                assert!(source.starts_with("src/") && !source.contains(".."));
                continue;
            }
            let path = Path::new("../..").join(p);
            assert!(path.is_file(), "missing audit source {}", path.display());
            println!("cargo:rerun-if-changed={}", path.display());
        }
        assert!(r
            .rust
            .split("::")
            .all(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_lowercase() || c == b'_')));
        code += &format!("ctx.run({:?}, {})?;\n", r.id, r.rust);
    }
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("schemas.json"),
        serde_json::to_vec(&schemas).unwrap(),
    )
    .unwrap();
    code += "Ok(())\n}\n";
    fs::write(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("readers.rs"),
        code,
    )
    .unwrap();
}
