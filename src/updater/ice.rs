use super::base::{BaseUpdater, UpdateResult};
use crate::types::*;
use anyhow::{Context, Result};

/// 雾凇方案更新器
pub struct IceUpdater {
    pub base: BaseUpdater,
}

impl IceUpdater {
    /// 检查方案更新 (雾凇所有文件在一个 release 里)
    pub async fn check_scheme_update(&self) -> Result<UpdateInfo> {
        let releases = self
            .base
            .client
            .fetch_github_releases(ICE_OWNER, ICE_REPO, "")
            .await?;

        BaseUpdater::find_update_info(&releases, "full.zip", None)
            .context("未找到雾凇方案文件: full.zip")
    }

    /// 更新方案
    pub async fn update_scheme(
        &self,
        config: &crate::types::Config,
        mut progress: impl FnMut(&str, f64),
    ) -> Result<UpdateResult> {
        progress("检查雾凇方案更新...", 0.05);

        let info = self.check_scheme_update().await?;
        let record_path = self.base.cache_dir.join("scheme_record.json");
        let local = BaseUpdater::load_record(&record_path);

        // 方案切换检测
        let scheme_switched = local
            .as_ref()
            .map(|r| r.name != "full.zip")
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
            name: "full.zip".into(),
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

        progress("雾凇方案更新完成", 1.0);
        Ok(UpdateResult {
            component: "方案".into(),
            old_version: local.map(|r| r.tag).unwrap_or_else(|| "未安装".into()),
            new_version: info.tag,
            success: true,
            message: "更新成功".into(),
        })
    }

    /// 检查词库更新
    pub async fn check_dict_update(&self) -> Result<UpdateInfo> {
        let releases = self
            .base
            .client
            .fetch_github_releases(ICE_OWNER, ICE_REPO, "")
            .await?;

        BaseUpdater::find_update_info(&releases, "all_dicts.zip", None)
            .context("未找到雾凇词库: all_dicts.zip")
    }

    /// 更新词库
    pub async fn update_dict(
        &self,
        config: &crate::types::Config,
        mut progress: impl FnMut(&str, f64),
    ) -> Result<UpdateResult> {
        progress("检查雾凇词库更新...", 0.05);

        let info = self.check_dict_update().await?;
        let record_path = self.base.cache_dir.join("dict_record.json");
        let local = BaseUpdater::load_record(&record_path);

        if !BaseUpdater::needs_update(local.as_ref(), &info) {
            progress("词库已是最新", 1.0);
            return Ok(BaseUpdater::success_result(
                "词库",
                &info.tag,
                &info.tag,
                "已是最新版本",
            ));
        }

        // 下载到 dicts 子目录
        let dict_dir = self.base.rime_dir.join("dicts");
        self.base
            .download_and_extract(&info, config, &dict_dir, &mut progress)
            .await?;

        // 保存记录
        progress("保存记录...", 0.95);
        let record = UpdateRecord {
            name: "all_dicts.zip".into(),
            update_time: info.update_time.clone(),
            tag: info.tag.clone(),
            apply_time: chrono::Utc::now().to_rfc3339(),
            sha256: info.sha256.clone(),
        };
        BaseUpdater::save_record(&record_path, &record)?;

        progress("雾凇词库更新完成", 1.0);
        Ok(UpdateResult {
            component: "词库".into(),
            old_version: local.map(|r| r.tag).unwrap_or_else(|| "未安装".into()),
            new_version: info.tag,
            success: true,
            message: "更新成功".into(),
        })
    }
}
