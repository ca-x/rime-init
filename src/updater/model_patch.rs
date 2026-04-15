use crate::types::Schema;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

type PatchDoc = HashMap<String, serde_yaml::Value>;

const PATCH_KEY: &str = "patch";
const MODEL_KEY: &str = "grammar/language_model";
const MODEL_VALUE: &str = "wanxiang-lts-zh-hans";

/// 为万象方案 patch 模型配置
///
/// 写入 `<schema_id>.custom.yaml`:
/// ```yaml
/// patch:
///   grammar/language_model: wanxiang-lts-zh-hans
/// ```
pub fn patch_model(rime_dir: &Path, schema: &Schema) -> Result<()> {
    let patch_file = patch_file_path(rime_dir, schema);
    let mut doc = load_patch_doc(&patch_file)?;

    let patch = doc
        .entry(PATCH_KEY.into())
        .or_insert_with(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));

    if let serde_yaml::Value::Mapping(mapping) = patch {
        mapping.insert(
            serde_yaml::Value::String(MODEL_KEY.into()),
            serde_yaml::Value::String(MODEL_VALUE.into()),
        );
    } else {
        anyhow::bail!("模型 patch 文件中的 `{PATCH_KEY}` 节不是映射类型");
    }

    write_patch_doc(&patch_file, &doc)?;

    println!("✅ 模型 patch 已写入: {}", patch_file.display());
    Ok(())
}

/// 移除模型 patch
pub fn unpatch_model(rime_dir: &Path, schema: &Schema) -> Result<()> {
    let patch_file = patch_file_path(rime_dir, schema);

    if !patch_file.exists() {
        return Ok(());
    }

    let mut doc = load_patch_doc(&patch_file)?;

    if let Some(patch) = doc.get_mut(PATCH_KEY) {
        if let serde_yaml::Value::Mapping(mapping) = patch {
            mapping.remove(serde_yaml::Value::String(MODEL_KEY.to_string()));
        } else {
            anyhow::bail!("模型 patch 文件中的 `{PATCH_KEY}` 节不是映射类型");
        }
    }

    write_patch_doc(&patch_file, &doc)?;

    println!("✅ 模型 patch 已移除");
    Ok(())
}

/// 检查模型 patch 是否已启用
pub fn is_model_patched(rime_dir: &Path, schema: &Schema) -> bool {
    let patch_file = patch_file_path(rime_dir, schema);

    match load_patch_doc(&patch_file) {
        Ok(doc) => has_model_patch(&doc),
        Err(e) => {
            eprintln!("⚠️ 读取模型 patch 状态失败: {e}");
            false
        }
    }
}

fn patch_file_path(rime_dir: &Path, schema: &Schema) -> std::path::PathBuf {
    rime_dir.join(format!("{}.custom.yaml", schema.schema_id()))
}

fn load_patch_doc(path: &Path) -> Result<PatchDoc> {
    if !path.exists() {
        return Ok(HashMap::new());
    }

    let data = std::fs::read_to_string(path)
        .with_context(|| format!("读取模型 patch 文件失败: {}", path.display()))?;
    serde_yaml::from_str(&data)
        .with_context(|| format!("解析模型 patch 文件失败: {}", path.display()))
}

fn write_patch_doc(path: &Path, doc: &PatchDoc) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let yaml = serde_yaml::to_string(doc)?;
    std::fs::write(path, yaml)?;
    Ok(())
}

fn has_model_patch(doc: &PatchDoc) -> bool {
    match doc.get(PATCH_KEY) {
        Some(serde_yaml::Value::Mapping(mapping)) => {
            mapping.contains_key(serde_yaml::Value::String(MODEL_KEY.into()))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_rime_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("snout-{name}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn patch_model_fails_fast_on_invalid_yaml() {
        let dir = temp_rime_dir("model-patch-invalid");
        let file = patch_file_path(&dir, &Schema::WanxiangBase);
        std::fs::write(&file, "patch: [broken").expect("write invalid yaml");

        let result = patch_model(&dir, &Schema::WanxiangBase);

        assert!(result.is_err());
        let err = result.expect_err("invalid yaml should fail");
        assert!(err.to_string().contains("解析模型 patch 文件失败"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn is_model_patched_returns_false_on_invalid_yaml() {
        let dir = temp_rime_dir("model-patch-detect-invalid");
        let file = patch_file_path(&dir, &Schema::WanxiangBase);
        std::fs::write(&file, "patch: [broken").expect("write invalid yaml");

        assert!(!is_model_patched(&dir, &Schema::WanxiangBase));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn patch_and_unpatch_model_round_trip() {
        let dir = temp_rime_dir("model-patch-roundtrip");

        patch_model(&dir, &Schema::WanxiangBase).expect("patch model");
        assert!(is_model_patched(&dir, &Schema::WanxiangBase));

        unpatch_model(&dir, &Schema::WanxiangBase).expect("unpatch model");
        assert!(!is_model_patched(&dir, &Schema::WanxiangBase));

        std::fs::remove_dir_all(&dir).ok();
    }
}
