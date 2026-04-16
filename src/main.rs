mod api;
mod config;
mod deployer;
mod fileutil;
mod i18n;
mod skin;
mod types;
mod ui;
mod updater;

use clap::Parser;
use i18n::{L10n, Lang};
use types::Schema;

#[derive(Parser, Debug)]
#[command(
    name = "snout",
    version,
    about = env!("CARGO_PKG_DESCRIPTION")
)]
struct Cli {
    /// 首次初始化模式 / First-time setup mode
    #[arg(long)]
    init: bool,

    /// 更新所有组件 / Update all components
    #[arg(long, short)]
    update: bool,

    /// 仅更新方案 / Update scheme only
    #[arg(long)]
    scheme: bool,

    /// 仅更新词库 / Update dictionary only
    #[arg(long)]
    dict: bool,

    /// 仅更新模型 / Update model only
    #[arg(long)]
    model: bool,

    /// 启用模型 patch / Enable model patch
    #[arg(long)]
    patch_model: bool,

    /// 使用 CNB 镜像 / Use CNB mirror
    #[arg(long)]
    mirror: bool,

    /// 代理地址 / Proxy address (socks5://host:port or http://host:port)
    #[arg(long)]
    proxy: Option<String>,

    /// 语言 / Language (zh/en)
    #[arg(long)]
    lang: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.init {
        ui::wizard::run_init_wizard().await?;
    } else if cli.update || cli.scheme || cli.dict || cli.model {
        let mut manager = config::Manager::new()?;

        // 应用 CLI 覆盖
        if cli.mirror {
            manager.config.use_mirror = true;
        }
        if let Some(ref proxy) = cli.proxy {
            manager.config.proxy_enabled = true;
            if proxy.starts_with("http://") {
                manager.config.proxy_type = "http".into();
                manager.config.proxy_address = proxy.trim_start_matches("http://").into();
            } else if proxy.starts_with("socks5://") {
                manager.config.proxy_type = "socks5".into();
                manager.config.proxy_address = proxy.trim_start_matches("socks5://").into();
            } else {
                manager.config.proxy_address = proxy.clone();
            }
        }
        if let Some(ref lang) = cli.lang {
            manager.config.language = lang.clone();
        }
        let t = L10n::new(Lang::from_str(&manager.config.language));

        let schema = manager.config.schema;
        let cache_dir = manager.cache_dir.clone();
        let rime_dir = manager.rime_dir.clone();

        if cli.update {
            updater::update_all(
                &schema,
                &manager.config,
                cache_dir,
                rime_dir,
                types::CancelSignal::new(),
                |event| {
                    print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                    std::io::Write::flush(&mut std::io::stdout()).ok();
                },
            )
            .await?;
            println!();
        } else if cli.scheme {
            let base = updater::BaseUpdater::new(&manager.config, cache_dir, rime_dir)?;
            if schema.is_wanxiang() {
                updater::wanxiang::WanxiangUpdater { base }
                    .update_scheme(&schema, &manager.config, None, |event| {
                        print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                    })
                    .await?;
            } else if schema == Schema::Ice {
                updater::ice::IceUpdater { base }
                    .update_scheme(&manager.config, None, |event| {
                        print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                    })
                    .await?;
            } else if schema == Schema::Frost {
                updater::frost::FrostUpdater { base }
                    .update_scheme(&manager.config, None, |event| {
                        print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                    })
                    .await?;
            } else {
                updater::mint::MintUpdater { base }
                    .update_scheme(&manager.config, None, |event| {
                        print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                    })
                    .await?;
            }
            println!();
        } else if cli.dict {
            if schema.dict_zip().is_some() {
                let base = updater::BaseUpdater::new(&manager.config, cache_dir, rime_dir)?;
                if schema.is_wanxiang() {
                    updater::wanxiang::WanxiangUpdater { base }
                        .update_dict(&schema, &manager.config, None, |event| {
                            print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                            std::io::Write::flush(&mut std::io::stdout()).ok();
                        })
                        .await?;
                } else {
                    updater::ice::IceUpdater { base }
                        .update_dict(&manager.config, None, |event| {
                            print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                            std::io::Write::flush(&mut std::io::stdout()).ok();
                        })
                        .await?;
                }
                println!();
            } else {
                eprintln!("{}", t.t("update.no_dict"));
            }
        } else if cli.model {
            if !schema.supports_model_patch() {
                eprintln!("{}", t.t("update.model_not_supported"));
                std::process::exit(1);
            } else {
                let base = updater::BaseUpdater::new(&manager.config, cache_dir, rime_dir.clone())?;
                updater::wanxiang::WanxiangUpdater { base }
                    .update_model(&manager.config, None, |event| {
                        print!("\r  [{:3.0}%] {}", event.progress * 100.0, event.detail);
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                    })
                    .await?;

                if cli.patch_model {
                    updater::model_patch::patch_model(
                        &rime_dir,
                        &schema,
                        Lang::from_str(&manager.config.language),
                    )?;
                }
                println!();
            }
        }
    } else {
        // 默认启动 TUI
        ui::app::run_tui().await?;
    }

    Ok(())
}
