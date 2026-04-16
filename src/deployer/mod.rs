use crate::i18n::{L10n, Lang};
use anyhow::Result;
use std::path::{Path, PathBuf};

/// 跨平台部署 Rime
pub fn deploy(lang: Lang) -> Result<()> {
    let t = L10n::new(lang);
    let engines = detect_engines();
    ensure_deployable_engine_set(!engines.is_empty(), &t)?;

    let mut success_count = 0usize;
    let mut failures = Vec::new();
    for engine in &engines {
        match deploy_to(engine, &t) {
            Ok(()) => success_count += 1,
            Err(e) => {
                eprintln!("⚠️  {} ({engine}): {e}", t.t("deploy.target_failed"));
                failures.push(format!("{engine}: {e}"));
            }
        }
    }

    finalize_deploy_result(success_count, failures, &t)
}

/// 部署到指定引擎
pub fn deploy_to(engine: &str, t: &L10n) -> Result<()> {
    match engine {
        #[cfg(target_os = "linux")]
        "fcitx5" => {
            let bin = find_binary("fcitx5-remote", t)?;
            std::process::Command::new(bin).arg("-r").spawn()?;
            println!("  ✅ {}", t.t("deploy.reloaded.fcitx5"));
        }
        #[cfg(target_os = "linux")]
        "ibus" => {
            let bin = find_binary("ibus", t)?;
            std::process::Command::new(bin)
                .args(["engine", "Rime"])
                .spawn()?;
            println!("  ✅ {}", t.t("deploy.reloaded.ibus"));
        }
        #[cfg(target_os = "macos")]
        "squirrel" => {
            let squirrel = "/Library/Input Methods/Squirrel.app/Contents/MacOS/Squirrel";
            if Path::new(squirrel).exists() {
                std::process::Command::new(squirrel)
                    .arg("--reload")
                    .spawn()?;
                println!("  ✅ {}", t.t("deploy.reloaded.squirrel"));
            }
        }
        #[cfg(target_os = "windows")]
        "weasel" => {
            let weasel = Path::new(r"C:\Program Files\Rime\weaselDeployer.exe");
            if weasel.exists() {
                std::process::Command::new(weasel).spawn()?;
                println!("  ✅ {}", t.t("deploy.reloaded.weasel"));
            }
        }
        _ => {}
    }
    Ok(())
}

/// 检测已安装的引擎
pub fn detect_engines() -> Vec<String> {
    let mut engines = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if has_binary("fcitx5-remote") {
            engines.push("fcitx5".into());
        }
        if has_binary("ibus") {
            engines.push("ibus".into());
        }
    }

    #[cfg(target_os = "macos")]
    {
        if Path::new("/Library/Input Methods/Squirrel.app").exists() {
            engines.push("squirrel".into());
        }
    }

    #[cfg(target_os = "windows")]
    {
        if Path::new(r"C:\Program Files\Rime").exists() {
            engines.push("weasel".into());
        }
    }

    engines
}

/// 获取主引擎数据目录
#[allow(dead_code)]
pub fn engine_data_dir(engine: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    match engine {
        #[cfg(target_os = "linux")]
        "fcitx5" => Some(dirs::data_dir()?.join("fcitx5/rime")),
        #[cfg(target_os = "linux")]
        "ibus" => Some(home.join(".config/ibus/rime")),
        #[cfg(target_os = "macos")]
        "squirrel" => Some(home.join("Library/Rime")),
        #[cfg(target_os = "windows")]
        "weasel" => {
            let appdata = std::env::var("APPDATA").ok()?;
            Some(PathBuf::from(appdata).join("Rime"))
        }
        _ => None,
    }
}

/// 同步 Rime 目录到所有已安装引擎的数据目录
#[allow(dead_code)]
pub fn sync_to_engines(
    src_dir: &Path,
    exclude_files: &[String],
    use_link: bool,
    lang: Lang,
) -> Result<()> {
    let t = L10n::new(lang);
    let engines = detect_engines();
    if engines.len() <= 1 {
        return Ok(());
    }

    let primary = engines.first().cloned().unwrap_or_default();
    let mut errors = Vec::new();

    for engine in &engines {
        if *engine == primary {
            continue;
        }
        if let Some(target) = engine_data_dir(engine) {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let sync_result = if use_link && cfg!(unix) {
                sync_via_symlink(src_dir, &target)
            } else {
                sync_dir_filtered(src_dir, &target, exclude_files)
            };
            if let Err(e) = sync_result {
                errors.push(format!("{engine}: {e}"));
            }
        }
    }

    if !errors.is_empty() {
        eprintln!(
            "⚠️ {}: {}",
            t.t("deploy.sync_partial_failed"),
            errors.join("; ")
        );
    }
    Ok(())
}

#[allow(dead_code)]
fn sync_dir_filtered(src: &Path, dst: &Path, exclude_files: &[String]) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // 跳过排除的文件
        if exclude_files.iter().any(|e| name_str == *e) {
            continue;
        }
        // 跳过 build 目录
        if name_str == "build" {
            continue;
        }

        let src_path = entry.path();
        let dst_path = dst.join(&name);

        if src_path.is_dir() {
            sync_dir_filtered(&src_path, &dst_path, exclude_files)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// 执行 hook 脚本
pub fn run_hook(hook_path: &str, phase: &str, lang: Lang) -> Result<()> {
    if hook_path.is_empty() {
        return Ok(());
    }

    let t = L10n::new(lang);
    let path = Path::new(hook_path);
    if !path.exists() {
        eprintln!("  ⚠️ {phase} {}: {hook_path}", t.t("deploy.hook_missing"));
        return Ok(());
    }

    println!("  🔧 {phase} {}: {hook_path}", t.t("deploy.hook_running"));
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg(hook_path)
        .status()?;

    if !status.success() {
        anyhow::bail!("{phase} {}: {hook_path}", t.t("deploy.hook_failed"));
    }
    Ok(())
}

// ── 辅助函数 ──

fn has_binary(name: &str) -> bool {
    which(name).is_some()
}

fn find_binary(name: &str, lang: &L10n) -> Result<PathBuf> {
    which(name).ok_or_else(|| anyhow::anyhow!("{}: {name}", lang.t("deploy.binary_not_found")))
}

fn which(name: &str) -> Option<PathBuf> {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim()))
}

#[cfg(unix)]
fn sync_via_symlink(src: &Path, target: &Path) -> Result<()> {
    if target.exists() || target.is_symlink() {
        let backup = target.with_extension("bak");
        if target.is_symlink() || target.is_file() {
            std::fs::remove_file(target)?;
        } else if target.is_dir() {
            if backup.exists() {
                if backup.is_dir() {
                    std::fs::remove_dir_all(&backup)?;
                } else {
                    std::fs::remove_file(&backup)?;
                }
            }
            std::fs::rename(target, &backup)?;
        }
    }

    std::os::unix::fs::symlink(src, target)?;
    Ok(())
}

fn ensure_deployable_engine_set(has_engines: bool, t: &L10n) -> Result<()> {
    if has_engines {
        Ok(())
    } else {
        anyhow::bail!("{}", t.t("deploy.no_engine_detected"));
    }
}

fn finalize_deploy_result(success_count: usize, failures: Vec<String>, t: &L10n) -> Result<()> {
    if success_count == 0 {
        anyhow::bail!(
            "{}: {}",
            t.t("deploy.all_engines_failed"),
            failures.join("; ")
        );
    }

    if !failures.is_empty() {
        eprintln!(
            "⚠️  {}: {}",
            t.t("deploy.partial_engines_failed"),
            failures.join("; ")
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("snout-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn rejects_empty_engine_set() {
        let t = L10n::new(Lang::Zh);
        assert!(ensure_deployable_engine_set(false, &t).is_err());
        assert!(ensure_deployable_engine_set(true, &t).is_ok());
    }

    #[test]
    fn fails_when_all_deployments_fail() {
        let t = L10n::new(Lang::Zh);
        let err = finalize_deploy_result(0, vec!["fcitx5: boom".into()], &t).unwrap_err();
        assert!(err.to_string().contains(t.t("deploy.all_engines_failed")));
    }

    #[test]
    fn succeeds_when_at_least_one_deployment_succeeds() {
        let t = L10n::new(Lang::Zh);
        assert!(finalize_deploy_result(1, Vec::new(), &t).is_ok());
        assert!(finalize_deploy_result(1, vec!["ibus: failed".into()], &t).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fcitx_engine_data_dir_uses_fcitx5_data_path() {
        let path = engine_data_dir("fcitx5").expect("fcitx5 dir");
        assert!(path.ends_with("fcitx5/rime"));
    }

    #[cfg(unix)]
    #[test]
    fn sync_via_symlink_does_not_create_backup_for_missing_target() {
        let base = temp_dir("deployer-link");
        let src = base.join("src");
        let target = base.join("target");
        std::fs::create_dir_all(&src).expect("create src dir");

        sync_via_symlink(&src, &target).expect("create symlink");

        assert!(target.is_symlink());
        assert!(!target.with_extension("bak").exists());

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn sync_dir_filtered_copies_files_and_skips_exclusions() {
        let base = temp_dir("deployer-copy");
        let src = base.join("src");
        let dst = base.join("dst");
        std::fs::create_dir_all(src.join("nested")).expect("create nested src dir");
        std::fs::create_dir_all(src.join("build")).expect("create build dir");
        std::fs::write(src.join("keep.txt"), "keep").expect("write keep");
        std::fs::write(src.join("skip.txt"), "skip").expect("write skip");
        std::fs::write(src.join("nested").join("child.txt"), "child").expect("write child");
        std::fs::write(src.join("build").join("artifact.txt"), "artifact").expect("write artifact");
        std::fs::create_dir_all(&dst).expect("create dst");
        std::fs::write(dst.join("preexisting.txt"), "stay").expect("write preexisting");

        sync_dir_filtered(&src, &dst, &["skip.txt".into()]).expect("sync dir");

        assert_eq!(
            std::fs::read_to_string(dst.join("keep.txt")).unwrap(),
            "keep"
        );
        assert_eq!(
            std::fs::read_to_string(dst.join("nested").join("child.txt")).unwrap(),
            "child"
        );
        assert!(!dst.join("skip.txt").exists());
        assert!(!dst.join("build").exists());
        assert_eq!(
            std::fs::read_to_string(dst.join("preexisting.txt")).unwrap(),
            "stay"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn run_hook_accepts_empty_and_missing_paths() {
        assert!(run_hook("", "pre-update", Lang::En).is_ok());
        assert!(run_hook("/definitely/missing/hook.sh", "pre-update", Lang::En).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn run_hook_reports_failure_for_nonzero_exit() {
        let base = temp_dir("hook-fail");
        let script = base.join("fail.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 7\n").expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }

        let err = run_hook(
            script.to_str().expect("script path"),
            "post-update",
            Lang::En,
        )
        .unwrap_err();
        assert!(err.to_string().contains("Hook execution failed"));

        std::fs::remove_dir_all(&base).ok();
    }

    #[cfg(unix)]
    #[test]
    fn run_hook_runs_successful_commands() {
        let base = temp_dir("hook-ok");
        let script = base.join("ok.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").expect("write script");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&script, perms).unwrap();
        }

        assert!(run_hook(
            script.to_str().expect("script path"),
            "post-update",
            Lang::En
        )
        .is_ok());

        std::fs::remove_dir_all(&base).ok();
    }
}
