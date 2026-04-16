use crate::config::Manager;
use crate::i18n::{L10n, Lang};
use crate::types::Schema;
use crate::updater;
use crate::updater::{UpdateComponent, UpdateEvent, UpdatePhase};
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

// ── 应用状态 ──
pub enum AppScreen {
    Menu,
    Updating,
    Result,
    SchemeSelector,
    SkinSelector,
    ConfigView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    Success,
    Partial,
    Failure,
    Cancelled,
}

pub struct App {
    pub should_quit: bool,
    pub screen: AppScreen,
    pub menu_selected: usize,
    pub scheme_selected: usize,
    pub skin_selected: usize,
    pub schema: Schema,
    pub rime_dir: String,
    pub config_path: String,
    pub t: L10n,
    // 更新状态
    pub update_msg: String,
    pub update_pct: f64,
    pub update_done: bool,
    pub update_results: Vec<String>,
    pub update_stage_lines: Vec<String>,
    update_outcome: Option<UpdateOutcome>,
    update_in_progress: bool,
    progress_rx: Option<mpsc::Receiver<UpdateEvent>>,
    result_rx: Option<mpsc::Receiver<UpdateTaskResult>>,
    update_task: Option<JoinHandle<()>>,
    cancel_signal: Option<crate::types::CancelSignal>,
    // 通知
    pub notification: Option<(String, Instant)>,
}

#[derive(Debug)]
enum UpdateTaskError {
    Cancelled,
    Failed(String),
}

#[derive(Debug)]
struct UpdateTaskResult {
    results: Result<Vec<updater::UpdateResult>, UpdateTaskError>,
}

#[derive(Clone)]
struct ResolvedUpdateContext {
    schema: Schema,
    config: crate::types::Config,
    cache_dir: std::path::PathBuf,
    rime_dir: std::path::PathBuf,
}

impl App {
    pub fn new(manager: &Manager) -> Self {
        let lang = Lang::from_str(&manager.config.language);
        Self {
            should_quit: false,
            screen: AppScreen::Menu,
            menu_selected: 0,
            scheme_selected: 0,
            skin_selected: 0,
            schema: manager.config.schema,
            rime_dir: manager.rime_dir.display().to_string(),
            config_path: manager.config_path.display().to_string(),
            t: L10n::new(lang),
            update_msg: String::new(),
            update_pct: 0.0,
            update_done: false,
            update_results: Vec::new(),
            update_stage_lines: Vec::new(),
            update_outcome: None,
            update_in_progress: false,
            progress_rx: None,
            result_rx: None,
            update_task: None,
            cancel_signal: None,
            notification: None,
        }
    }

    /// 动态菜单项 (i18n)
    pub fn menu_items(&self) -> Vec<(&str, &str)> {
        vec![
            ("1", self.t.t("menu.update_all")),
            ("2", self.t.t("menu.update_scheme")),
            ("3", self.t.t("menu.update_dict")),
            ("4", self.t.t("menu.update_model")),
            ("5", self.t.t("menu.model_patch")),
            ("6", self.t.t("menu.skin_patch")),
            ("7", self.t.t("menu.switch_scheme")),
            ("8", self.t.t("menu.config")),
            ("Q", self.t.t("menu.quit")),
        ]
    }

    pub fn notify(&mut self, msg: impl Into<String>) {
        self.notification = Some((msg.into(), Instant::now()));
    }

    fn current_hint(&self) -> String {
        match self.screen {
            AppScreen::Updating => format!(
                "{}  q/Esc {}",
                self.t.t("hint.wait"),
                self.t.t("hint.cancel")
            ),
            AppScreen::Result => format!("Enter/Esc {}", self.t.t("hint.back")),
            AppScreen::SchemeSelector | AppScreen::SkinSelector => {
                format!(
                    "↑↓/jk {}  Enter {}  Esc {}",
                    self.t.t("hint.navigate"),
                    self.t.t("hint.confirm"),
                    self.t.t("hint.back")
                )
            }
            AppScreen::ConfigView => format!("Enter/Esc {}", self.t.t("hint.back")),
            AppScreen::Menu => format!(
                "↑↓/jk {}  Enter {}  q/Esc {}",
                self.t.t("hint.navigate"),
                self.t.t("hint.confirm"),
                self.t.t("hint.back")
            ),
        }
    }
}

fn model_update_supported(schema: Schema) -> bool {
    schema.supports_model_patch()
}

fn skin_patch_target(rime_dir: &std::path::Path) -> anyhow::Result<std::path::PathBuf> {
    if cfg!(target_os = "windows") {
        Ok(rime_dir.join("weasel.custom.yaml"))
    } else if cfg!(target_os = "macos") {
        Ok(rime_dir.join("squirrel.custom.yaml"))
    } else {
        let t = L10n::new(Lang::Zh);
        anyhow::bail!("{}", t.t("skin.not_supported"))
    }
}

fn resolve_update_context(
    app: &App,
    manager: &Manager,
    mode: &UpdateMode,
) -> anyhow::Result<ResolvedUpdateContext> {
    let schema = app.schema;
    if matches!(mode, UpdateMode::Model) && !model_update_supported(schema) {
        anyhow::bail!("{}", app.t.t("update.model_not_supported"));
    }

    Ok(ResolvedUpdateContext {
        schema,
        config: manager.config.clone(),
        cache_dir: manager.cache_dir.clone(),
        rime_dir: manager.rime_dir.clone(),
    })
}

// ── 主入口 ──
pub async fn run_tui() -> Result<()> {
    let manager = Manager::new()?;
    let mut app = App::new(&manager);

    // 终端设置
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut app, &manager).await;

    // 恢复终端
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    manager: &Manager,
) -> Result<()> {
    loop {
        let mut progress_events = Vec::new();
        if let Some(rx) = &app.progress_rx {
            while let Ok(event) = rx.try_recv() {
                progress_events.push(event);
            }
        }
        for event in progress_events {
            app.update_msg = event.detail.clone();
            app.update_pct = event.progress.clamp(0.0, 1.0);
            upsert_stage_line(app, &event);
        }
        let mut finished_results = Vec::new();
        if let Some(rx) = &app.result_rx {
            while let Ok(event) = rx.try_recv() {
                finished_results.push(event.results);
            }
        }
        for results in finished_results {
            finish_update(app, results);
        }

        terminal.draw(|f| ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match app.screen {
                    AppScreen::Menu => handle_menu_key(app, key.code, manager).await?,
                    AppScreen::Updating => handle_updating_key(app, key.code),
                    AppScreen::Result => handle_result_key(app, key.code),
                    AppScreen::SchemeSelector => handle_scheme_key(app, key.code, manager)?,
                    AppScreen::SkinSelector => handle_skin_key(app, key.code, manager)?,
                    AppScreen::ConfigView => handle_config_key(app, key.code),
                }
            }
        }

        // 清除过期通知
        if let Some((_, t)) = &app.notification {
            if t.elapsed() > Duration::from_secs(6) {
                app.notification = None;
            }
        }

        if app.should_quit {
            return Ok(());
        }
    }
}

// ── 按键处理 ──

async fn handle_menu_key(app: &mut App, key: KeyCode, manager: &Manager) -> Result<()> {
    match key {
        KeyCode::Up | KeyCode::Char('k') => {
            app.menu_selected = app.menu_selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.menu_selected < app.menu_items().len() - 1 {
                app.menu_selected += 1;
            }
        }
        KeyCode::Enter | KeyCode::Char('1'..='8') => {
            let idx = match key {
                KeyCode::Char(c) => c.to_digit(10).unwrap_or(0) as usize,
                _ => app.menu_selected + 1,
            };
            if let Some(reason) = menu_unavailable_reason(app, idx) {
                app.notify(reason);
                return Ok(());
            }
            match idx {
                1 => start_update(app, manager, UpdateMode::All).await?,
                2 => start_update(app, manager, UpdateMode::Scheme).await?,
                3 => start_update(app, manager, UpdateMode::Dict).await?,
                4 => start_update(app, manager, UpdateMode::Model).await?,
                5 => {
                    // Model patch toggle
                    app.screen = AppScreen::Result;
                    app.update_results.clear();
                    app.update_stage_lines.clear();
                    if app.schema.supports_model_patch() {
                        let patched = updater::model_patch::is_model_patched(
                            std::path::Path::new(&app.rime_dir),
                            &app.schema,
                            app.t.lang(),
                        );
                        if patched {
                            if let Err(e) = updater::model_patch::unpatch_model(
                                std::path::Path::new(&app.rime_dir),
                                &app.schema,
                                app.t.lang(),
                            ) {
                                app.update_results.push(format!("❌ {e}"));
                            } else {
                                app.update_results
                                    .push(format!("✅ {}", app.t.t("patch.model.disabled")));
                            }
                        } else {
                            if let Err(e) = updater::model_patch::patch_model(
                                std::path::Path::new(&app.rime_dir),
                                &app.schema,
                                app.t.lang(),
                            ) {
                                app.update_results.push(format!("❌ {e}"));
                            } else {
                                app.update_results
                                    .push(format!("✅ {}", app.t.t("patch.model.enabled")));
                            }
                        }
                    } else {
                        app.update_results
                            .push(app.t.t("patch.model.not_supported").into());
                    }
                    app.update_msg = app.t.t("menu.model_patch").into();
                    app.update_done = true;
                    app.update_outcome = Some(UpdateOutcome::Success);
                }
                6 => {
                    app.skin_selected = 0;
                    app.screen = AppScreen::SkinSelector;
                }
                7 => {
                    app.scheme_selected = current_schema_index(app.schema);
                    app.screen = AppScreen::SchemeSelector;
                }
                8 => app.screen = AppScreen::ConfigView,
                _ => {}
            }
        }
        KeyCode::Char('q') | KeyCode::Esc => {
            app.should_quit = true;
        }
        _ => {}
    }
    Ok(())
}

fn handle_result_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = AppScreen::Menu;
            app.update_done = false;
            app.update_in_progress = false;
            app.update_pct = 0.0;
            app.update_stage_lines.clear();
            app.progress_rx = None;
            app.result_rx = None;
            app.update_task = None;
            app.cancel_signal = None;
        }
        _ => {}
    }
}

fn handle_updating_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            if app.update_in_progress {
                if let Some(cancel) = &app.cancel_signal {
                    cancel.cancel();
                }
                app.update_msg = app.t.t("update.cancelling").into();
                upsert_stage_line(
                    app,
                    &UpdateEvent {
                        component: UpdateComponent::Hook,
                        phase: UpdatePhase::Cancelling,
                        progress: app.update_pct,
                        detail: app.t.t("update.cancelling").into(),
                    },
                );
            }
        }
        _ => {}
    }
}

fn handle_scheme_key(app: &mut App, key: KeyCode, _manager: &Manager) -> Result<()> {
    let schemas = Schema::all();
    match key {
        KeyCode::Up | KeyCode::Char('k') => {
            app.scheme_selected = app.scheme_selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.scheme_selected < schemas.len() - 1 {
                app.scheme_selected += 1;
            }
        }
        KeyCode::Enter => {
            if let Some(s) = schemas.get(app.scheme_selected) {
                app.schema = *s;
                let mut m = Manager::new()?;
                m.config.schema = *s;
                m.save()?;
                app.notify(format!(
                    "{}: {}",
                    app.t.t("scheme.switched"),
                    s.display_name_lang(app.t.lang())
                ));
            }
            app.screen = AppScreen::Menu;
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = AppScreen::Menu;
        }
        _ => {}
    }
    Ok(())
}

fn handle_skin_key(app: &mut App, key: KeyCode, _manager: &Manager) -> Result<()> {
    let skins = crate::skin::builtin::list_available_skins(app.t.lang());
    match key {
        KeyCode::Up | KeyCode::Char('k') => {
            app.skin_selected = app.skin_selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.skin_selected < skins.len() - 1 {
                app.skin_selected += 1;
            }
        }
        KeyCode::Enter => {
            if let Some((key, name)) = skins.get(app.skin_selected) {
                let rime_dir = std::path::Path::new(&app.rime_dir);
                match skin_patch_target(rime_dir) {
                    Ok(patch) => {
                        if let Err(e) =
                            crate::skin::patch::write_skin_presets(&patch, &[key.as_str()])
                        {
                            app.notify(format!("❌ {e}"));
                        } else if let Err(e) = crate::skin::patch::set_default_skin(&patch, key) {
                            app.notify(format!("❌ {e}"));
                        } else {
                            app.notify(format!("✅ {}: {name}", app.t.t("skin.applied")));
                        }
                    }
                    Err(_) => {
                        let msg = app.t.t("skin.not_supported").to_string();
                        app.notify(msg);
                    }
                }
            }
            app.screen = AppScreen::Menu;
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.screen = AppScreen::Menu;
        }
        _ => {}
    }
    Ok(())
}

fn handle_config_key(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Enter => {
            app.screen = AppScreen::Menu;
        }
        _ => {}
    }
}

// ── 更新调度 ──

enum UpdateMode {
    All,
    Scheme,
    Dict,
    Model,
}

async fn start_update(app: &mut App, manager: &Manager, mode: UpdateMode) -> Result<()> {
    app.screen = AppScreen::Updating;
    app.update_msg = app.t.t("update.checking").into();
    app.update_pct = 0.0;
    app.update_done = false;
    app.update_in_progress = true;
    app.update_outcome = None;
    app.update_results.clear();
    app.update_stage_lines.clear();
    app.update_task = None;
    app.cancel_signal = None;

    let context = match resolve_update_context(app, manager, &mode) {
        Ok(context) => context,
        Err(e) => {
            app.update_results.push(format!("❌ {}", e));
            app.update_msg = app.t.t("update.failed").into();
            app.update_pct = 1.0;
            app.update_done = true;
            app.update_in_progress = false;
            app.update_outcome = Some(UpdateOutcome::Failure);
            app.screen = AppScreen::Result;
            return Ok(());
        }
    };

    let (progress_tx, progress_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    app.progress_rx = Some(progress_rx);
    app.result_rx = Some(result_rx);
    let lang = app.t.lang();
    let cancel_signal = crate::types::CancelSignal::new();
    app.cancel_signal = Some(cancel_signal.clone());

    let handle = tokio::spawn(async move {
        let results = run_update_task(context, mode, lang, cancel_signal.clone(), move |event| {
            let _ = progress_tx.send(event);
        })
        .await;
        let _ = result_tx.send(UpdateTaskResult {
            results: match results {
                Ok(value) => Ok(value),
                Err(e) if e.is::<crate::types::UpdateCancelled>() => {
                    Err(UpdateTaskError::Cancelled)
                }
                Err(e) => Err(UpdateTaskError::Failed(e.to_string())),
            },
        });
    });
    app.update_task = Some(handle);

    Ok(())
}

async fn run_update_task(
    context: ResolvedUpdateContext,
    mode: UpdateMode,
    lang: Lang,
    cancel: crate::types::CancelSignal,
    mut progress: impl FnMut(UpdateEvent) + Send + 'static,
) -> Result<Vec<updater::UpdateResult>> {
    let t = L10n::new(lang);
    match mode {
        UpdateMode::All => {
            updater::update_all(
                &context.schema,
                &context.config,
                context.cache_dir,
                context.rime_dir,
                cancel,
                &mut progress,
            )
            .await
        }
        UpdateMode::Scheme => {
            let base = updater::BaseUpdater::new(
                &context.config,
                context.cache_dir.clone(),
                context.rime_dir.clone(),
            )?;
            if context.schema.is_wanxiang() {
                updater::wanxiang::WanxiangUpdater { base }
                    .update_scheme(&context.schema, &context.config, Some(&cancel), |event| {
                        progress(event)
                    })
                    .await
                    .map(|r| vec![r])
            } else if context.schema == Schema::Ice {
                updater::ice::IceUpdater { base }
                    .update_scheme(&context.config, Some(&cancel), &mut progress)
                    .await
                    .map(|r| vec![r])
            } else if context.schema == Schema::Frost {
                updater::frost::FrostUpdater { base }
                    .update_scheme(&context.config, Some(&cancel), &mut progress)
                    .await
                    .map(|r| vec![r])
            } else {
                updater::mint::MintUpdater { base }
                    .update_scheme(&context.config, Some(&cancel), &mut progress)
                    .await
                    .map(|r| vec![r])
            }
        }
        UpdateMode::Dict => {
            if context.schema.dict_zip().is_none() {
                Ok(vec![updater::UpdateResult {
                    component: t.t("update.dict").into(),
                    old_version: "-".into(),
                    new_version: "-".into(),
                    success: false,
                    message: t.t("update.no_dict").into(),
                }])
            } else {
                let base = updater::BaseUpdater::new(
                    &context.config,
                    context.cache_dir,
                    context.rime_dir,
                )?;
                if context.schema.is_wanxiang() {
                    updater::wanxiang::WanxiangUpdater { base }
                        .update_dict(&context.schema, &context.config, Some(&cancel), |event| {
                            progress(event)
                        })
                        .await
                        .map(|r| vec![r])
                } else {
                    updater::ice::IceUpdater { base }
                        .update_dict(&context.config, Some(&cancel), &mut progress)
                        .await
                        .map(|r| vec![r])
                }
            }
        }
        UpdateMode::Model => {
            let base = updater::BaseUpdater::new(
                &context.config,
                context.cache_dir,
                context.rime_dir.clone(),
            )?;
            let wx = updater::wanxiang::WanxiangUpdater { base };
            let r = wx
                .update_model(&context.config, Some(&cancel), &mut progress)
                .await?;
            let mut v = vec![r];
            if context.config.model_patch_enabled && context.schema.supports_model_patch() {
                progress(UpdateEvent {
                    component: UpdateComponent::ModelPatch,
                    phase: UpdatePhase::Applying,
                    progress: 0.96,
                    detail: t.t("menu.model_patch").into(),
                });
                cancel.checkpoint()?;
                if let Err(e) =
                    updater::model_patch::patch_model(&context.rime_dir, &context.schema, lang)
                {
                    v.push(updater::UpdateResult {
                        component: t.t("update.component.model_patch").into(),
                        old_version: "?".into(),
                        new_version: "?".into(),
                        success: false,
                        message: e.to_string(),
                    });
                } else {
                    v.push(updater::UpdateResult {
                        component: t.t("update.component.model_patch").into(),
                        old_version: "-".into(),
                        new_version: t.t("patch.model.enabled").into(),
                        success: true,
                        message: t.t("patch.model.enabled").into(),
                    });
                    progress(UpdateEvent {
                        component: UpdateComponent::ModelPatch,
                        phase: UpdatePhase::Finished,
                        progress: 1.0,
                        detail: t.t("patch.model.enabled").into(),
                    });
                }
            }
            Ok(v)
        }
    }
}

fn finish_update(app: &mut App, results: Result<Vec<updater::UpdateResult>, UpdateTaskError>) {
    match results {
        Ok(rs) => {
            let all_success = rs.iter().all(|r| r.success);
            let any_success = rs.iter().any(|r| r.success);
            for r in &rs {
                let icon = if r.success { "✅" } else { "❌" };
                app.update_results
                    .push(format!("{icon} {} - {}", r.component, r.message));
            }
            app.update_msg = if all_success {
                app.t.t("update.complete").into()
            } else if any_success {
                app.t.t("update.partial").into()
            } else {
                app.t.t("update.failed").into()
            };
            app.update_outcome = Some(if all_success {
                UpdateOutcome::Success
            } else if any_success {
                UpdateOutcome::Partial
            } else {
                UpdateOutcome::Failure
            });
        }
        Err(UpdateTaskError::Cancelled) => {
            app.update_results
                .push(format!("⚠️ {}", app.t.t("update.cancelled")));
            app.update_msg = app.t.t("update.cancelled").into();
            app.update_outcome = Some(UpdateOutcome::Cancelled);
            upsert_stage_line(
                app,
                &UpdateEvent {
                    component: UpdateComponent::Hook,
                    phase: UpdatePhase::Cancelled,
                    progress: 1.0,
                    detail: app.t.t("update.cancelled").into(),
                },
            );
        }
        Err(UpdateTaskError::Failed(e)) => {
            app.update_results
                .push(format!("❌ {}: {e}", app.t.t("update.failed")));
            app.update_msg = app.t.t("update.failed").into();
            app.update_outcome = Some(UpdateOutcome::Failure);
        }
    }

    app.update_pct = 1.0;
    app.update_done = true;
    app.update_in_progress = false;
    app.screen = AppScreen::Result;
    app.progress_rx = None;
    app.result_rx = None;
    app.update_task = None;
    app.cancel_signal = None;
}

fn upsert_stage_line(app: &mut App, event: &UpdateEvent) {
    let label = format!(
        "{}: {}",
        component_label(app, event.component),
        phase_label(app, event.phase)
    );
    let line = format!("{label} - {}", event.detail);
    if let Some(existing) = app
        .update_stage_lines
        .iter_mut()
        .find(|entry| entry.starts_with(&label))
    {
        *existing = line;
    } else {
        app.update_stage_lines.push(line);
    }
}

fn component_label(app: &App, component: UpdateComponent) -> &str {
    match component {
        UpdateComponent::Scheme => app.t.t("update.scheme"),
        UpdateComponent::Dict => app.t.t("update.dict"),
        UpdateComponent::Model => app.t.t("update.model"),
        UpdateComponent::ModelPatch => app.t.t("update.component.model_patch"),
        UpdateComponent::Deploy => app.t.t("update.component.deploy"),
        UpdateComponent::Sync => app.t.t("update.component.sync"),
        UpdateComponent::Hook => app.t.t("update.component.hook"),
    }
}

fn phase_label(app: &App, phase: UpdatePhase) -> &str {
    match phase {
        UpdatePhase::Starting => app.t.t("update.checking"),
        UpdatePhase::Checking => app.t.t("update.checking"),
        UpdatePhase::Downloading => app.t.t("update.downloading"),
        UpdatePhase::Verifying => app.t.t("update.verifying"),
        UpdatePhase::Extracting => app.t.t("update.extracting"),
        UpdatePhase::Saving => app.t.t("update.saving"),
        UpdatePhase::Applying => app.t.t("menu.model_patch"),
        UpdatePhase::Deploying => app.t.t("update.deploying"),
        UpdatePhase::Syncing => app.t.t("update.syncing"),
        UpdatePhase::Running => app.t.t("hint.wait"),
        UpdatePhase::Cancelling => app.t.t("update.cancelling"),
        UpdatePhase::Cancelled => app.t.t("update.cancelled"),
        UpdatePhase::Finished => app.t.t("update.complete"),
    }
}

fn current_schema_index(schema: Schema) -> usize {
    Schema::all()
        .iter()
        .position(|candidate| *candidate == schema)
        .unwrap_or(0)
}

fn menu_unavailable_reason(app: &App, idx: usize) -> Option<String> {
    match idx {
        3 if app.schema.dict_zip().is_none() => Some(format!(
            "{}: {}",
            app.t.t("hint.unavailable"),
            app.t.t("update.no_dict")
        )),
        6 if cfg!(target_os = "linux") => Some(format!(
            "{}: {}",
            app.t.t("hint.unavailable"),
            app.t.t("skin.not_supported")
        )),
        _ => None,
    }
}

// ── 渲染 ──

fn ui(f: &mut Frame, app: &App) {
    let size = f.area();

    // 主布局: header + body + footer
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // header
            Constraint::Min(8),    // body
            Constraint::Length(3), // footer
        ])
        .split(size);

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " snout ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("v{}  ", env!("CARGO_PKG_VERSION")),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            app.schema.display_name_lang(app.t.lang()),
            Style::default().fg(Color::Green),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan)),
    )
    .alignment(Alignment::Center);
    f.render_widget(header, chunks[0]);

    // Body - 根据屏幕渲染
    match app.screen {
        AppScreen::Menu => render_menu(f, chunks[1], app),
        AppScreen::Updating => render_updating(f, chunks[1], app),
        AppScreen::Result => render_result(f, chunks[1], app),
        AppScreen::SchemeSelector => render_scheme_selector(f, chunks[1], app),
        AppScreen::SkinSelector => render_skin_selector(f, chunks[1], app),
        AppScreen::ConfigView => render_config(f, chunks[1], app),
    }

    // Footer / 通知
    let footer_text = if let Some((msg, _)) = &app.notification {
        vec![Span::styled(
            format!(" 💡 {msg}"),
            Style::default().fg(Color::Yellow),
        )]
    } else {
        vec![Span::styled(
            format!(" {}", app.current_hint()),
            Style::default().fg(Color::White),
        )]
    };
    let footer =
        Paragraph::new(Line::from(footer_text)).block(Block::default().borders(Borders::TOP));
    f.render_widget(footer, chunks[2]);
}

fn render_menu(f: &mut Frame, area: Rect, app: &App) {
    let menu_items = app.menu_items();
    let items: Vec<ListItem> = menu_items
        .iter()
        .enumerate()
        .map(|(i, (key, label))| {
            let idx = i + 1;
            let unavailable = menu_unavailable_reason(app, idx).is_some();
            let style = if unavailable {
                Style::default().fg(Color::DarkGray)
            } else if i == 5 || i == 6 {
                Style::default().fg(Color::Magenta)
            } else {
                Style::default().fg(Color::White)
            };
            let mut line = vec![
                Span::styled(format!("  {key}. "), Style::default().fg(Color::Cyan)),
                Span::styled(*label, style),
            ];
            if unavailable {
                line.push(Span::styled(
                    format!("  [{}]", app.t.t("hint.unavailable")),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            ListItem::new(Line::from(line))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(
                    format!(" {} ", app.t.t("menu.title")),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.menu_selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn render_updating(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(3)])
        .split(area);

    let msg = Paragraph::new(Line::from(vec![
        Span::styled("  ⏳ ", Style::default().fg(Color::Yellow)),
        Span::styled(&app.update_msg, Style::default().fg(Color::White)),
    ]))
    .block(Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" {} ", app.t.t("update.checking")),
        Style::default().fg(Color::Yellow),
    )));
    f.render_widget(msg, chunks[0]);

    let gauge = Gauge::default()
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" {} ", app.t.t("update.progress"))),
        )
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
        .ratio(app.update_pct)
        .label(format!("{:.0}%", app.update_pct * 100.0));
    f.render_widget(gauge, chunks[1]);
}

fn render_result(f: &mut Frame, area: Rect, app: &App) {
    let title = if app.update_done {
        format!(" {} ", app.t.t("menu.done"))
    } else {
        format!(" {} ", app.t.t("menu.result"))
    };
    let (accent, status_color) = match app.update_outcome {
        Some(UpdateOutcome::Success) => (Color::Green, Color::Green),
        Some(UpdateOutcome::Partial) => (Color::Yellow, Color::Yellow),
        Some(UpdateOutcome::Failure) => (Color::Red, Color::Red),
        Some(UpdateOutcome::Cancelled) => (Color::DarkGray, Color::Yellow),
        None => (Color::Yellow, Color::Yellow),
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                &app.update_msg,
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
    ];

    for stage in &app.update_stage_lines {
        lines.push(Line::from(vec![
            Span::styled("  • ", Style::default().fg(Color::DarkGray)),
            Span::styled(stage, Style::default().fg(Color::DarkGray)),
        ]));
    }
    if !app.update_stage_lines.is_empty() {
        lines.push(Line::from(""));
    }

    for r in &app.update_results {
        lines.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(r, Style::default().fg(Color::White)),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        format!("  {}", app.t.t("result.back_to_menu")),
        Style::default().fg(Color::DarkGray),
    )]));

    let p = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(accent))
                .title(Span::styled(&title, Style::default().fg(accent))),
        )
        .wrap(Wrap { trim: true });
    f.render_widget(p, area);
}

fn render_scheme_selector(f: &mut Frame, area: Rect, app: &App) {
    let schemas = Schema::all();
    let items: Vec<ListItem> = schemas
        .iter()
        .map(|s| {
            let prefix = if *s == app.schema { " ● " } else { " ○ " };
            let style = if s.is_wanxiang() {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::Green)
            };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::Yellow)),
                Span::styled(s.display_name_lang(app.t.lang()), style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(Span::styled(
                    format!(" {} ", app.t.t("scheme.select_prompt")),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.scheme_selected.min(schemas.len() - 1)));
    f.render_stateful_widget(list, area, &mut state);
}

fn render_skin_selector(f: &mut Frame, area: Rect, app: &App) {
    let skins = crate::skin::builtin::list_available_skins(app.t.lang());
    let items: Vec<ListItem> = skins
        .iter()
        .map(|(key, name)| {
            ListItem::new(Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(name.as_str(), Style::default().fg(Color::White)),
                Span::styled(format!(" ({key})"), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title(Span::styled(
                    format!(" {} ", app.t.t("skin.select_prompt")),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
        )
        .highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▸ ");

    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.skin_selected.min(skins.len().saturating_sub(1))));
    f.render_stateful_widget(list, area, &mut state);
}

fn render_config(f: &mut Frame, area: Rect, app: &App) {
    let manager = Manager::new().ok();
    let engines = manager
        .as_ref()
        .map(|_| crate::config::detect_installed_engines().join(", "))
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| app.t.t("config.none").into());
    let language = if app.t.lang() == Lang::Zh {
        app.t.t("config.lang.zh")
    } else {
        app.t.t("config.lang.en")
    };
    let config = manager.map(|m| m.config).unwrap_or_default();
    let lines = vec![
        Line::from(vec![Span::styled(
            format!("  {}:", app.t.t("config.runtime_section")),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.current_scheme")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                app.schema.display_name_lang(app.t.lang()),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.detected_engines")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(engines, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.language_label")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(language, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("  {}:", app.t.t("config.features_section")),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.mirror_label")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                if config.use_mirror {
                    app.t.t("config.enabled")
                } else {
                    app.t.t("config.disabled")
                },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.proxy_label")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                if config.proxy_enabled {
                    &config.proxy_address
                } else {
                    app.t.t("config.none")
                },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.model_patch_label")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                if config.model_patch_enabled {
                    app.t.t("config.enabled")
                } else {
                    app.t.t("config.disabled")
                },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.engine_sync_label")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                if config.engine_sync_enabled {
                    app.t.t("config.enabled")
                } else {
                    app.t.t("config.disabled")
                },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.sync_strategy_label")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(
                if config.engine_sync_use_link {
                    app.t.t("config.sync_link")
                } else {
                    app.t.t("config.sync_copy")
                },
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("  {}:", app.t.t("config.paths_section")),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.rime_dir")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(&app.rime_dir, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled(
                format!("  {}: ", app.t.t("config.config_file")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(&app.config_path, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            format!("  {}", app.t.t("config.back")),
            Style::default().fg(Color::DarkGray),
        )]),
    ];

    let p = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue))
            .title(Span::styled(
                format!(" {} ", app.t.t("config.title")),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
    );
    f.render_widget(p, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_update_supported_for_all_supported_schemas() {
        assert!(model_update_supported(Schema::WanxiangBase));
        assert!(model_update_supported(Schema::WanxiangMoqi));
        assert!(model_update_supported(Schema::Ice));
        assert!(model_update_supported(Schema::Frost));
        assert!(model_update_supported(Schema::Mint));
    }

    #[test]
    fn skin_patch_target_matches_platform_convention() {
        let base = std::path::Path::new("/tmp/rime");
        #[cfg(target_os = "windows")]
        assert_eq!(
            skin_patch_target(base).unwrap(),
            base.join("weasel.custom.yaml")
        );

        #[cfg(target_os = "macos")]
        assert_eq!(
            skin_patch_target(base).unwrap(),
            base.join("squirrel.custom.yaml")
        );

        #[cfg(target_os = "linux")]
        assert!(skin_patch_target(base).is_err());
    }

    #[test]
    fn current_schema_index_tracks_active_schema() {
        assert_eq!(current_schema_index(Schema::WanxiangBase), 0);
        assert_eq!(current_schema_index(Schema::Mint), Schema::all().len() - 1);
    }

    #[test]
    fn menu_unavailable_reason_blocks_dict_for_schema_without_separate_dict() {
        let manager = Manager::new().expect("manager");
        let mut app = App::new(&manager);
        app.schema = Schema::Mint;
        assert!(menu_unavailable_reason(&app, 3).is_some());
    }

    #[test]
    fn handle_updating_key_marks_update_as_cancelled() {
        let manager = Manager::new().expect("manager");
        let mut app = App::new(&manager);
        app.screen = AppScreen::Updating;
        app.update_in_progress = true;
        app.cancel_signal = Some(crate::types::CancelSignal::new());

        handle_updating_key(&mut app, KeyCode::Esc);

        assert!(matches!(app.screen, AppScreen::Updating));
        assert_eq!(app.update_msg, app.t.t("update.cancelling"));
    }
}
