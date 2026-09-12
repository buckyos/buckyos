use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let metadata_dir = manifest_dir.join("driver_metadata");
    println!("cargo:rerun-if-changed={}", metadata_dir.display());

    let groups = [
        ("models", "CatalogKind::ModelDriver"),
        ("providers", "CatalogKind::ProviderRules"),
        ("known-providers", "CatalogKind::KnownProvider"),
    ];
    let mut entries = Vec::new();
    for (directory, kind) in groups {
        let path = metadata_dir.join(directory);
        for entry in fs::read_dir(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        {
            let entry = entry.expect("metadata directory entry");
            let file = entry.path();
            if file.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let relative = file
                .strip_prefix(&manifest_dir)
                .expect("metadata belongs to crate")
                .to_string_lossy()
                .replace('\\', "/");
            entries.push((relative, kind));
        }
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let mut generated =
        String::from("const BUILTIN_METADATA_DOCUMENTS: &[(CatalogKind, &[u8])] = &[\n");
    for (relative, kind) in entries {
        generated.push_str(&format!(
            "    ({kind}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{relative}\"))),\n"
        ));
    }
    generated.push_str("];\n");
    let output = Path::new(&env::var("OUT_DIR").expect("out dir")).join("builtin_metadata.rs");
    fs::write(output, generated).expect("write builtin metadata manifest");

    generate_protocol_plugins(&manifest_dir);
}

fn generate_protocol_plugins(manifest_dir: &Path) {
    let plugin_dir = manifest_dir.join("src/protocol/plugins");
    println!("cargo:rerun-if-changed={}", plugin_dir.display());
    let mut plugins = fs::read_dir(&plugin_dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", plugin_dir.display()))
        .map(|entry| entry.expect("protocol plugin directory entry").path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("rs"))
        .collect::<Vec<_>>();
    plugins.sort();
    let mut generated = String::new();
    let mut names = Vec::new();
    for (index, path) in plugins.iter().enumerate() {
        let name = format!("adapter_plugin_{index}");
        generated.push_str(&format!(
            "#[path = {:?}] mod {name};\n",
            path.to_string_lossy()
        ));
        names.push(name);
    }
    generated.push_str(
        "pub(crate) fn register_builtin_adapter_plugins(registry: &mut CodecRegistry) -> ProtocolResultValue<()> {\n",
    );
    generated.push_str("    let plugins: &[&dyn ProtocolAdapterPlugin] = &[\n");
    for name in &names {
        generated.push_str(&format!("        &{name}::Plugin,\n"));
    }
    generated.push_str(
        "    ];\n    for plugin in plugins { plugin.register(registry)?; }\n    Ok(())\n}\n",
    );
    let output = Path::new(&env::var("OUT_DIR").expect("out dir")).join("protocol_plugins.rs");
    fs::write(output, generated).expect("write protocol plugin registry");
}
