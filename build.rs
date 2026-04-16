use std::collections::BTreeSet;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let theme_root = manifest_dir.join("assets/fcitx5-themes");
    println!("cargo:rerun-if-changed={}", theme_root.display());

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("out dir"));
    let out_path = out_dir.join("fcitx5_theme_manifest.rs");

    let mut theme_names = BTreeSet::new();
    let mut files = Vec::new();
    if theme_root.exists() {
        collect_theme_files(
            &manifest_dir,
            &theme_root,
            &theme_root,
            &mut theme_names,
            &mut files,
        );
    }
    files.sort();

    let mut output = String::new();
    writeln!(&mut output, "pub const FCITX5_THEME_NAMES: &[&str] = &[").expect("write names");
    for name in &theme_names {
        writeln!(&mut output, "    {:?},", name).expect("write theme name");
    }
    writeln!(&mut output, "];").expect("close names");

    writeln!(
        &mut output,
        "pub const FCITX5_THEME_FILES: &[(&str, &str, &[u8])] = &["
    )
    .expect("write files");
    for (theme_name, rel_path, manifest_rel_path) in files {
        writeln!(
            &mut output,
            "    ({:?}, {:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{}\"))),",
            theme_name, rel_path, manifest_rel_path
        )
        .expect("write theme file");
    }
    writeln!(&mut output, "];").expect("close files");

    fs::write(out_path, output).expect("write manifest");
}

fn collect_theme_files(
    manifest_dir: &Path,
    theme_root: &Path,
    current_dir: &Path,
    theme_names: &mut BTreeSet<String>,
    files: &mut Vec<(String, String, String)>,
) {
    let entries = fs::read_dir(current_dir).expect("read assets directory");
    for entry in entries {
        let entry = entry.expect("read asset entry");
        let path = entry.path();
        if path.is_dir() {
            collect_theme_files(manifest_dir, theme_root, &path, theme_names, files);
            continue;
        }

        let rel_path = path.strip_prefix(theme_root).expect("theme relative path");
        let mut components = rel_path.components();
        let theme_name = components
            .next()
            .expect("theme directory")
            .as_os_str()
            .to_string_lossy()
            .to_string();
        if theme_name.is_empty() {
            continue;
        }
        theme_names.insert(theme_name.clone());

        let theme_rel_path = rel_path
            .strip_prefix(Path::new(&theme_name))
            .expect("theme file relative path")
            .to_string_lossy()
            .replace('\\', "/");
        let manifest_rel_path = path
            .strip_prefix(manifest_dir)
            .expect("manifest relative path")
            .to_string_lossy()
            .replace('\\', "/");

        files.push((theme_name, theme_rel_path, manifest_rel_path));
    }
}
