use super::base::{BaseUpdater, UpdateResult};
use crate::types::*;
use anyhow::{Context, Result};

/// 白霜方案更新器
pub struct FrostUpdater {
    pub base: BaseUpdater,
}

impl FrostUpdater {
    /// 检查方案更新
    pub async fn check_scheme_update(&self) -> Result<UpdateInfo> {
        let releases = self
            .base
            .client
            .fetch_github_releases(FROST_OWNER, FROST_REPO, "")
            .await?;

        BaseUpdater::find_update_info(&releases, "rime-frost-schemas.zip", None)
            .context("未找到白霜方案文件: rime-frost-schemas.zip")
    }

    /// 更新方案
    pub async fn update_scheme(
        &self,
        config: &crate::types::Config,
        mut progress: impl FnMut(&str, f64),
    ) -> Result<UpdateResult> {
        progress("检查白霜方案更新...", 0.05);

        let info = self.check_scheme_update().await?;
        let record_path = self.base.cache_dir.join("scheme_record.json");
        let local = BaseUpdater::load_record(&record_path);

        // 方案切换检测
        let scheme_switched = local
            .as_ref()
            .map(|r| r.name != "rime-frost-schemas.zip")
            .unwrap_or(false);

        if !scheme_switched && !BaseUpdater::needs_update(local.as_ref(), &info) {
            progress("方案已是最新", 1.0);
            return Ok(BaseUpdater::success_result(
                "方案",
                &info.tag,
                &info.tag,
                "已是最新版本",
            ));
        }

        if scheme_switched {
            progress("检测到方案切换，重新下载...", 0.05);
        }

        // 下载并解压
        self.base
            .download_and_extract(&info, config, &self.base.rime_dir, &mut progress)
            .await?;

        // 保存记录
        progress("保存记录...", 0.95);
        let record = UpdateRecord {
            name: "rime-frost-schemas.zip".into(),
            update_time: info.update_time.clone(),
            tag: info.tag.clone(),
            apply_time: chrono::Utc::now().to_rfc3339(),
            sha256: info.sha256.clone(),
        };
        BaseUpdater::save_record(&record_path, &record)?;

        // 清理 build 目录
        let build_dir = self.base.rime_dir.join("build");
        if build_dir.exists() {
            let _ = std::fs::remove_dir_all(&build_dir);
        }

        progress("白霜方案更新完成", 1.0);
        Ok(UpdateResult {
            component: "方案".into(),
            old_version: local.map(|r| r.tag).unwrap_or_else(|| "未安装".into()),
            new_version: info.tag,
            success: true,
            message: "更新成功".into(),
        })
    }

    // 白霜词库内嵌在方案 zip 中，不需要单独更新
}
