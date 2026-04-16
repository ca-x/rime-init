use crate::i18n::{L10n, Lang};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::PathBuf;

include!(concat!(env!("OUT_DIR"), "/fcitx5_theme_manifest.rs"));

pub fn builtin_theme_choices() -> Vec<(String, String)> {
    FCITX5_THEME_NAMES
        .iter()
        .map(|name| ((*name).to_string(), (*name).to_string()))
        .collect()
}

pub fn builtin_themes_available() -> bool {
    !FCITX5_THEME_NAMES.is_empty()
}

pub fn theme_supported(installed_engines: &[String]) -> bool {
    #[cfg(target_os = "linux")]
    {
        installed_engines.iter().any(|engine| engine == "fcitx5")
            || which_exists("fcitx5")
            || which_exists("fcitx5-remote")
            || fcitx_theme_root_path()
                .map(|path| path.exists())
                .unwrap_or(false)
            || fcitx_classicui_config_path()
                .map(|path| path.exists())
                .unwrap_or(false)
    }

    #[cfg(not(target_os = "linux"))]
    {
        let _ = installed_engines;
        false
    }
}

pub fn installed_theme_names() -> Result<HashSet<String>> {
    let root = fcitx_theme_root_path()?;
    if !root.exists() {
        return Ok(HashSet::new());
    }

    let mut themes = HashSet::new();
    for entry in std::fs::read_dir(root).context("read fcitx5 theme root")? {
        let entry = entry.context("read fcitx5 theme entry")?;
        if entry.path().is_dir() {
            themes.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(themes)
}

pub fn current_theme() -> Result<Option<String>> {
    let config_path = fcitx_classicui_config_path()?;
    if !config_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(config_path).context("read fcitx5 classicui config")?;
    Ok(read_theme_key(&content))
}

pub fn apply_theme(theme_name: &str, lang: Lang) -> Result<()> {
    install_theme(theme_name)?;
    write_theme_setting(theme_name)?;
    reload_theme(lang)
}

fn install_theme(theme_name: &str) -> Result<()> {
    if !FCITX5_THEME_NAMES.contains(&theme_name) {
        anyhow::bail!("unknown fcitx5 theme: {theme_name}");
    }

    let root = fcitx_theme_root_path()?;
    let target_dir = root.join(theme_name);
    if target_dir.exists() {
        std::fs::remove_dir_all(&target_dir).context("remove existing fcitx5 theme")?;
    }
    std::fs::create_dir_all(&target_dir).context("create fcitx5 theme directory")?;

    let mut installed_any = false;
    for (theme, rel_path, bytes) in FCITX5_THEME_FILES {
        if *theme != theme_name {
            continue;
        }
        installed_any = true;
        let path = target_dir.join(rel_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create fcitx5 theme parent")?;
        }
        std::fs::write(path, bytes).context("write fcitx5 theme file")?;
    }

    if !installed_any {
        anyhow::bail!("embedded fcitx5 theme has no files: {theme_name}");
    }

    Ok(())
}

fn write_theme_setting(theme_name: &str) -> Result<()> {
    let config_path = fcitx_classicui_config_path()?;
    let content = if config_path.exists() {
        std::fs::read_to_string(&config_path).context("read classicui config before update")?
    } else {
        String::new()
    };
    let updated = upsert_key_value(&content, "Theme", theme_name);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).context("create classicui config dir")?;
    }
    std::fs::write(config_path, updated).context("write classicui config")?;
    Ok(())
}

fn reload_theme(lang: Lang) -> Result<()> {
    if reload_via_qdbus6().is_ok() {
        return Ok(());
    }
    if reload_via_fcitx5_remote().is_ok() {
        return Ok(());
    }

    let t = L10n::new(lang);
    crate::deployer::deploy_to("fcitx5", &t)
}

fn reload_via_qdbus6() -> Result<()> {
    if !which_exists("qdbus6") {
        anyhow::bail!("qdbus6 unavailable");
    }

    let status = std::process::Command::new("qdbus6")
        .args([
            "org.fcitx.Fcitx5",
            "/controller",
            "org.fcitx.Fcitx.Controller1.ReloadAddonConfig",
            "classicui",
        ])
        .status()
        .context("run qdbus6")?;
    if !status.success() {
        anyhow::bail!("qdbus6 reload failed");
    }
    Ok(())
}

fn reload_via_fcitx5_remote() -> Result<()> {
    if !which_exists("fcitx5-remote") {
        anyhow::bail!("fcitx5-remote unavailable");
    }

    let status = std::process::Command::new("fcitx5-remote")
        .arg("-r")
        .status()
        .context("run fcitx5-remote")?;
    if !status.success() {
        anyhow::bail!("fcitx5-remote reload failed");
    }
    Ok(())
}

fn read_theme_key(content: &str) -> Option<String> {
    for line in content.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "Theme" {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn upsert_key_value(content: &str, key: &str, value: &str) -> String {
    let mut lines = Vec::new();
    let mut found = false;

    for line in content.lines() {
        if let Some((line_key, _)) = line.split_once('=') {
            if line_key.trim() == key {
                lines.push(format!("{key}={value}"));
                found = true;
                continue;
            }
        }
        lines.push(line.to_string());
    }

    if !found {
        lines.push(format!("{key}={value}"));
    }

    let mut output = lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    output
}

fn fcitx_theme_root_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("missing home")?;
    Ok(home.join(".local/share/fcitx5/themes"))
}

fn fcitx_classicui_config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("missing home")?;
    Ok(home.join(".config/fcitx5/conf/classicui.conf"))
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_theme_manifest_is_not_empty() {
        assert!(builtin_themes_available());
    }

    #[test]
    fn read_theme_key_extracts_theme_value() {
        assert_eq!(
            read_theme_key("[Groups/0]\nTheme=OriLight\nUseDarkTheme=False\n"),
            Some("OriLight".into())
        );
    }

    #[test]
    fn upsert_key_value_replaces_existing_theme() {
        let output = upsert_key_value("Theme=Old\nUseDarkTheme=False\n", "Theme", "New");
        assert_eq!(output, "Theme=New\nUseDarkTheme=False\n");
    }

    #[test]
    fn upsert_key_value_appends_missing_theme() {
        let output = upsert_key_value("UseDarkTheme=False\n", "Theme", "OriDark");
        assert_eq!(output, "UseDarkTheme=False\nTheme=OriDark\n");
    }
}
